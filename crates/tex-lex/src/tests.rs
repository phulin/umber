use super::{
    ConditionFrameSummary, ConditionKind, ConditionLimb, InputFrame, InputFrameSummary,
    InputSource, InputStack, LexError, Lexer, LexerState, LineEvent, LineReader, LiteralSpanPolicy,
    MacroArguments, MemoryInput, TokenListReplayKind, load_next_line,
};
use tex_state::env::banks::IntParam;
use tex_state::ids::{OriginListId, TokenListId};
#[cfg(feature = "expansion-stats")]
use tex_state::meaning::Meaning;
use tex_state::provenance::{InsertedOriginKind, OriginRecord};
use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};
use tex_state::{ExpansionState, ProvenanceResolver, TracedTokenList, Universe};

mod input_lines;

#[test]
fn nested_alignment_resume_preserves_outer_align_state() {
    let mut input = InputStack::new(MemoryInput::new(""));
    input.begin_alignment();
    input.set_alignment_state(0);
    input.begin_alignment_cell(None, TokenListId::EMPTY, 7);
    let left = TracedTokenWord::pack(char_token('{', Catcode::BeginGroup), OriginId::UNKNOWN);
    for _ in 0..2 {
        assert!(!input.intercept_alignment_token(
            left,
            super::AlignmentTokenDelivery::LeftBrace,
            None,
            7,
        ));
    }

    let suspended = input.suspend_alignment_cell();
    input.resume_alignment_cell(suspended);
    let cr = TracedTokenWord::pack(char_token('x', Catcode::Escape), OriginId::UNKNOWN);

    assert!(!input.intercept_alignment_token(
        cr,
        super::AlignmentTokenDelivery::Other,
        Some(super::AlignmentTerminator::Cr),
        7,
    ));
}

#[test]
fn active_alignment_predicate_tracks_scanner_levels_not_only_cells() {
    let mut input = InputStack::new(MemoryInput::new(""));
    assert!(!input.has_active_alignment());

    input.begin_alignment();
    assert!(input.has_active_alignment());
    assert!(!input.has_active_alignment_cell());

    input.finish_alignment();
    assert!(!input.has_active_alignment());
}

#[test]
fn alignment_undo_bookkeeping_ignores_ordinary_deliveries() {
    let mut input = InputStack::new(MemoryInput::new(""));
    input.begin_alignment();
    input.set_alignment_state(0);
    input.begin_alignment_cell(None, TokenListId::EMPTY, 7);
    let ordinary = TracedTokenWord::pack(char_token('x', Catcode::Other), OriginId::UNKNOWN);

    for _ in 0..10_000 {
        assert!(!input.intercept_alignment_token(
            ordinary,
            super::AlignmentTokenDelivery::Other,
            None,
            7,
        ));
    }

    assert_eq!(
        input
            .alignment_inputs
            .last()
            .expect("active alignment")
            .align_state,
        0
    );

    let left = TracedTokenWord::pack(char_token('{', Catcode::BeginGroup), OriginId::UNKNOWN);
    assert!(!input.intercept_alignment_token(
        left,
        super::AlignmentTokenDelivery::LeftBrace,
        None,
        7,
    ));
    assert_eq!(
        input
            .alignment_inputs
            .last()
            .expect("active alignment")
            .align_state,
        1
    );
    input.undo_alignment_token_delivery(left);
    assert_eq!(
        input
            .alignment_inputs
            .last()
            .expect("active alignment")
            .align_state,
        0
    );

    let right = TracedTokenWord::pack(char_token('}', Catcode::EndGroup), OriginId::UNKNOWN);
    assert!(!input.intercept_alignment_token(
        right,
        super::AlignmentTokenDelivery::RightBrace,
        None,
        7,
    ));
    assert_eq!(
        input
            .alignment_inputs
            .last()
            .expect("active alignment")
            .align_state,
        -1
    );
    input.undo_alignment_token_delivery(right);
    assert_eq!(
        input
            .alignment_inputs
            .last()
            .expect("active alignment")
            .align_state,
        0
    );

    let mut stores = Universe::new();
    let symbol = stores.intern("brace-alias");
    let control_sequence = TracedTokenWord::pack(Token::Cs(symbol.symbol()), OriginId::UNKNOWN);
    input.undo_alignment_token_delivery(control_sequence);
    assert_eq!(
        input
            .alignment_inputs
            .last()
            .expect("active alignment")
            .align_state,
        0
    );
}

fn condition_context() -> TracedTokenWord {
    TracedTokenWord::pack(char_token('i', Catcode::Escape), OriginId::UNKNOWN)
}

#[test]
fn traced_memory_source_registers_before_delivery_and_survives_frame_pop() {
    let mut stores = Universe::new();
    let mut lexer = Lexer::new(MemoryInput::new("hello"));
    let first = lexer
        .next_traced_token(&mut stores)
        .expect("traced source operation succeeds")
        .expect("source token");
    while lexer
        .next_traced_token(&mut stores)
        .expect("traced source operation succeeds")
        .is_some()
    {}

    assert!(stores.source_position(super::SourceId::new(0), 0).is_ok());
    let rendered = ProvenanceResolver::new(&stores).render_diagnostic("boom", Some(first.origin()));
    assert!(rendered.contains("<source 0>:1:1"));
    assert!(rendered.contains("hello"));
}

#[test]
fn empty_memory_source_is_registered_even_without_delivered_tokens() {
    let mut stores = Universe::new();
    stores.set_int_param(IntParam::END_LINE_CHAR, -1);
    let mut lexer = Lexer::new(MemoryInput::new(""));
    assert!(
        lexer
            .next_traced_token(&mut stores)
            .expect("traced source operation succeeds")
            .is_none()
    );
    let anchor = stores
        .source_position(super::SourceId::new(0), 0)
        .expect("empty source anchor is live");
    assert!(
        stores
            .source_span(anchor, anchor)
            .expect("traced source operation succeeds")
            .is_empty()
    );
}

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
fn empty_lines_append_endlinechar_event() {
    let mut stores = Universe::new();
    stores.set_int_param(IntParam::END_LINE_CHAR, 13);
    let mut reader = LineReader::new(MemoryInput::new("   \n\nx\n"));

    assert_eq!(
        reader
            .next_event(&stores)
            .expect("memory input should read"),
        Some(LineEvent::Text("\r".to_owned()))
    );
    assert_eq!(
        reader
            .next_event(&stores)
            .expect("memory input should read"),
        Some(LineEvent::Text("\r".to_owned()))
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
fn blank_line_endlinechar_uses_current_catcode() {
    let mut stores = Universe::new();
    stores.set_int_param(IntParam::END_LINE_CHAR, 13);
    stores.set_catcode('\r', Catcode::Other);
    let mut lexer = Lexer::new(MemoryInput::new("\n"));

    assert_eq!(
        collect_tokens(&mut lexer, &mut stores),
        vec![char_token('\r', Catcode::Other)]
    );
    assert_eq!(lexer.frame().state(), LexerState::MidLine);
}

#[test]
fn blank_line_endlinechar_catcode_changes_before_line_load() {
    let mut stores = Universe::new();
    stores.set_int_param(IntParam::END_LINE_CHAR, 13);
    let mut lexer = Lexer::new(MemoryInput::new("a\n\n"));

    assert_eq!(
        lexer.next_token(&mut stores).expect("first token"),
        Some(char_token('a', Catcode::Letter))
    );
    assert_eq!(
        lexer.next_token(&mut stores).expect("first line ending"),
        Some(char_token(' ', Catcode::Space))
    );

    stores.set_catcode('\r', Catcode::Active);

    assert_eq!(
        collect_tokens(&mut lexer, &mut stores),
        vec![char_token('\r', Catcode::Active)]
    );
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
fn control_space_enters_skipping_blanks_state() {
    let mut stores = Universe::new();
    stores.set_int_param(IntParam::END_LINE_CHAR, 13);
    let mut lexer = Lexer::new(MemoryInput::new("a\\   b"));

    assert_eq!(
        collect_tokens(&mut lexer, &mut stores),
        vec![
            char_token('a', Catcode::Letter),
            cs_token(&mut stores, " "),
            char_token('b', Catcode::Letter),
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
    match lexer.next_traced_token(&mut stores) {
        Err(error @ LexError::InvalidCharacter { ch: '?', .. }) => {
            let context = error.source_context();
            assert_eq!(context.source_id(), tex_state::SourceId::new(0));
            assert_eq!(context.byte_offset(), 2);
            assert_eq!(context.byte_end(), 3);
            assert_eq!(context.line(), 1);
            assert_eq!(context.column(), 2);
            let rendered = ProvenanceResolver::new(&stores)
                .render_diagnostic_site("invalid", error.diagnostic_site());
            assert!(rendered.contains("  1 | a!?"), "{rendered}");
            assert!(rendered.contains("  ^"), "{rendered}");
        }
        other => panic!("expected invalid-character error, got {other:?}"),
    }
}

#[test]
fn readonly_missing_control_sequence_retains_source_context() {
    let mut stores = Universe::new();
    stores.set_int_param(IntParam::END_LINE_CHAR, 13);
    let mut input = InputStack::new(MemoryInput::new("\n"));

    let error = input
        .next_token_readonly(&stores)
        .expect_err("readonly lexing cannot intern the inserted par token");
    match error {
        LexError::MissingControlSequence { name, context, .. } => {
            assert_eq!(name, "par");
            assert_eq!(context.source_id(), tex_state::SourceId::new(0));
            assert_eq!(context.byte_offset(), 0);
            assert_eq!(context.line(), 1);
            assert_eq!(context.column(), 0);
        }
        other => panic!("expected missing-control-sequence error, got {other:?}"),
    }
}

#[test]
fn input_failure_retains_next_line_source_context() {
    #[derive(Debug)]
    struct FailingInput(Option<tex_state::WorldError>);

    impl InputSource for FailingInput {
        fn read_line(&mut self) -> Result<Option<super::PhysicalLine>, super::InputSourceError> {
            Err(self
                .0
                .take()
                .expect("failing input is read only once")
                .into())
        }
    }

    let mut stores = Universe::new();
    let world_error = stores
        .world_mut()
        .read_file(std::path::Path::new("missing-lex-input.tex"))
        .expect_err("test input should be absent");
    let mut input = InputStack::new(FailingInput(Some(world_error)));

    let error = input
        .next_traced_token(&mut stores)
        .expect_err("source read should fail");
    let LexError::Input { context, .. } = error else {
        panic!("expected input error");
    };
    assert_eq!(context.source_id(), tex_state::SourceId::new(0));
    assert_eq!(context.byte_offset(), 0);
    assert_eq!(context.line(), 1);
    assert_eq!(context.column(), 0);
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
fn superscript_notation_reprocesses_a_generated_superscript_character() {
    let mut stores = Universe::new();
    stores.set_int_param(IntParam::END_LINE_CHAR, 13);
    stores.set_catcode('q', Catcode::Superscript);
    let mut lexer = Lexer::new(MemoryInput::new("qq5e^5cbox10"));
    let tokens = collect_tokens(&mut lexer, &mut stores);

    assert_eq!(
        tokens[0],
        Token::Cs(stores.symbol("box").expect("control word").symbol())
    );
    assert_eq!(tokens[1], char_token('1', Catcode::Other));
    assert_eq!(tokens[2], char_token('0', Catcode::Other));
}

#[test]
fn traced_source_origins_use_token_start_coordinates() {
    let mut stores = Universe::new();
    stores.set_int_param(IntParam::END_LINE_CHAR, -1);
    let mut lexer = Lexer::new(MemoryInput::new("aé\\foo ^^41"));

    let first = lexer
        .next_traced_token(&mut stores)
        .expect("first token")
        .expect("first token");
    assert_eq!(first.token(), Some(char_token('a', Catcode::Letter)));
    assert_source_origin(&stores, first.origin(), 0, 1, 0);

    let second = lexer
        .next_traced_token(&mut stores)
        .expect("second token")
        .expect("second token");
    assert_eq!(second.token(), Some(char_token('é', Catcode::Other)));
    assert_source_origin(&stores, second.origin(), 1, 1, 1);

    let control = lexer
        .next_traced_token(&mut stores)
        .expect("control sequence")
        .expect("control sequence");
    assert_eq!(control.token(), Some(cs_token(&mut stores, "foo")));
    assert_source_origin(&stores, control.origin(), 3, 1, 2);

    let superscript = lexer
        .next_traced_token(&mut stores)
        .expect("superscript token")
        .expect("superscript token");
    assert_eq!(superscript.token(), Some(char_token('A', Catcode::Letter)));
    assert_source_origin(&stores, superscript.origin(), 8, 1, 7);
}

#[test]
fn control_sequences_and_transformed_input_retain_exact_physical_spellings() {
    let mut stores = Universe::new();
    stores.set_int_param(IntParam::END_LINE_CHAR, -1);
    let mut lexer = Lexer::new(MemoryInput::new("\\foo ^^41"));
    let control = lexer
        .next_traced_token(&mut stores)
        .expect("control sequence")
        .expect("control sequence");
    let transformed = lexer
        .next_traced_token(&mut stores)
        .expect("transformed token")
        .expect("transformed token");

    let control =
        ProvenanceResolver::new(&stores).render_diagnostic("control", Some(control.origin()));
    let transformed = ProvenanceResolver::new(&stores)
        .render_diagnostic("transformed", Some(transformed.origin()));
    assert!(control.contains("^^^^"), "{control}");
    assert!(transformed.contains("    ^^^^"), "{transformed}");
}

#[test]
fn source_range_join_requires_same_live_direct_frame_proofs() {
    let mut stores = Universe::new();
    stores.set_int_param(IntParam::END_LINE_CHAR, -1);
    let mut input = InputStack::new(MemoryInput::new("12"));
    let first = input
        .next_traced_token(&mut stores)
        .expect("first delivery")
        .expect("first token");
    let first_proof = input
        .take_direct_source_delivery(first)
        .expect("source delivery proof");
    let last = input
        .next_traced_token(&mut stores)
        .expect("last delivery")
        .expect("last token");
    let last_proof = input
        .take_direct_source_delivery(last)
        .expect("source delivery proof");
    let joined = input
        .join_direct_source_deliveries(&mut stores, first_proof, last_proof)
        .expect("same live frame joins");
    let OriginRecord::SourceSpan(span) = stores.origin(joined) else {
        panic!("joined delivery should allocate a source span");
    };
    assert_eq!(
        span.hi(),
        stores
            .source_position(super::SourceId::new(0), 2)
            .expect("exclusive end")
    );

    let list = stores.intern_token_list(&[char_token('x', Catcode::Letter)]);
    input.push_token_list(list, TokenListReplayKind::Inserted);
    let replayed = input
        .next_traced_token(&mut stores)
        .expect("replay")
        .expect("replayed token");
    assert!(input.take_direct_source_delivery(replayed).is_none());
}

#[test]
fn ordinary_source_scalars_append_no_provenance_records() {
    let mut stores = Universe::new();
    let mut lexer = Lexer::new(MemoryInput::new("aé"));
    let before = stores.provenance_stats();

    let ascii = lexer
        .next_traced_token(&mut stores)
        .expect("valid token")
        .expect("ASCII token");
    let after_ascii = stores.provenance_stats();
    let utf8 = lexer
        .next_traced_token(&mut stores)
        .expect("valid token")
        .expect("UTF-8 token");
    let after_utf8 = stores.provenance_stats();

    assert!(matches!(
        stores.origin(ascii.origin()),
        OriginRecord::SourceSpan(_)
    ));
    assert!(matches!(
        stores.origin(utf8.origin()),
        OriginRecord::SourceSpan(_)
    ));
    assert_eq!(after_ascii.origin_records(), before.origin_records());
    assert_eq!(after_utf8.origin_records(), before.origin_records());
    assert_eq!(after_utf8.source_regions(), 1);
}

#[test]
fn physical_byte_coordinates_preserve_crlf_trailing_spaces_and_utf8() {
    let mut stores = Universe::new();
    stores.set_int_param(IntParam::END_LINE_CHAR, 13);
    let mut lexer = Lexer::new(MemoryInput::new("é  \r\nx"));

    let utf8 = lexer
        .next_traced_token(&mut stores)
        .expect("UTF-8 token")
        .expect("UTF-8 token");
    assert_source_origin(&stores, utf8.origin(), 0, 1, 0);

    let summary = lexer.input_summary();
    let [InputFrameSummary::Source { source, .. }] = summary.frames() else {
        panic!("expected source frame");
    };
    assert_eq!(source.normalized_line(), "é\r");
    assert_eq!(source.line_byte_offset(), 2);
    assert_eq!(source.physical_content_end(), 4);
    assert_eq!(source.terminator_start(), 4);
    assert_eq!(source.terminator_end(), 6);
    assert_eq!(source.normalized_end_anchor(), 2);
    assert_eq!(source.synthetic_endline_start(), Some(2));
    assert_eq!(source.next_source_offset(), 6);

    let endline = lexer
        .next_traced_token(&mut stores)
        .expect("endline token")
        .expect("endline token");
    let parent = assert_inserted_origin(&stores, endline.origin(), InsertedOriginKind::EndLine);
    assert_source_origin(&stores, parent, 2, 1, 1);

    let next = lexer
        .next_traced_token(&mut stores)
        .expect("next-line token")
        .expect("next-line token");
    assert_source_origin(&stores, next.origin(), 6, 2, 0);
}

#[test]
fn missing_final_newline_and_comments_keep_physical_coordinates() {
    let mut stores = Universe::new();
    stores.set_int_param(IntParam::END_LINE_CHAR, -1);
    let mut lexer = Lexer::new(MemoryInput::new("a%ignored\r\nb"));

    let first = lexer
        .next_traced_token(&mut stores)
        .expect("first token")
        .expect("first token");
    assert_source_origin(&stores, first.origin(), 0, 1, 0);
    let second = lexer
        .next_traced_token(&mut stores)
        .expect("second token")
        .expect("second token");
    assert_source_origin(&stores, second.origin(), 11, 2, 0);
}

#[test]
fn failed_superscript_transform_restores_byte_cursor_and_column() {
    let mut stores = Universe::new();
    stores.set_int_param(IntParam::END_LINE_CHAR, -1);
    let mut lexer = Lexer::new(MemoryInput::new("é^^"));

    assert_eq!(
        lexer.next_token(&mut stores).expect("UTF-8 token"),
        Some(char_token('é', Catcode::Other))
    );
    assert_eq!(
        lexer.next_token(&mut stores).expect("first superscript"),
        Some(char_token('^', Catcode::Superscript))
    );
    let summary = lexer.input_summary();
    let [InputFrameSummary::Source { source, .. }] = summary.frames() else {
        panic!("expected source frame");
    };
    assert_eq!(source.line_byte_offset(), 3);
    assert_eq!(source.column(), 2);
    assert_eq!(
        lexer.next_token(&mut stores).expect("second superscript"),
        Some(char_token('^', Catcode::Superscript))
    );
}

#[test]
fn input_summary_restores_source_allocator_and_unicode_superscript_mode() {
    let mut stores = Universe::new();
    stores.set_int_param(IntParam::END_LINE_CHAR, -1);
    let mut input = InputStack::new(MemoryInput::new("a"));
    assert_eq!(
        input.push_source(MemoryInput::new("")),
        tex_state::SourceId::new(1)
    );
    input.set_unicode_superscript_notation(false);
    assert_eq!(
        input.next_token(&mut stores).expect("outer token"),
        Some(char_token('a', Catcode::Letter))
    );

    let summary = input.summary();
    assert_eq!(summary.next_source_id(), 2);
    assert!(!summary.unicode_superscript_notation());
    let mut restored =
        InputStack::from_summary(&summary, |_, _, _| Ok::<_, ()>(MemoryInput::new("")))
            .expect("summary restores");
    assert_eq!(
        restored.push_source(MemoryInput::new("")),
        tex_state::SourceId::new(2)
    );
    assert!(!restored.summary().unicode_superscript_notation());
}

#[test]
fn nested_sources_keep_independent_physical_offsets() {
    let mut stores = Universe::new();
    stores.set_int_param(IntParam::END_LINE_CHAR, -1);
    let mut input = InputStack::new(MemoryInput::new("outer"));
    let nested = input.push_source(MemoryInput::new("é"));

    let inner = input
        .next_traced_token(&mut stores)
        .expect("nested token")
        .expect("nested token");
    assert_source_origin_for(&stores, inner.origin(), nested, 0, 1, 0);
    let outer = input
        .next_traced_token(&mut stores)
        .expect("outer token")
        .expect("outer token");
    assert_source_origin_for(
        &stores,
        outer.origin(),
        tex_state::SourceId::new(0),
        0,
        1,
        0,
    );
}

#[test]
fn long_single_line_coordinates_advance_without_prefix_rescanning_state() {
    let mut stores = Universe::new();
    stores.set_int_param(IntParam::END_LINE_CHAR, -1);
    let source = "a".repeat(64 * 1024);
    let mut input = InputStack::new(MemoryInput::new(source));
    for _ in 0..(64 * 1024 - 1) {
        input.next_token(&mut stores).expect("long-line token");
    }
    let final_token = input
        .next_traced_token(&mut stores)
        .expect("final long-line token")
        .expect("final long-line token");
    assert_source_origin(&stores, final_token.origin(), 65_535, 1, 65_535);
}

#[test]
fn invalid_world_utf8_reports_exact_physical_byte_range() {
    let mut stores = Universe::new();
    stores
        .world_mut()
        .set_memory_file("invalid.tex", vec![b'a', 0xF0, 0x28, 0x8C, 0x28])
        .expect("seed invalid input");
    let content = stores
        .world_mut()
        .read_file("invalid.tex")
        .expect("read invalid input bytes");
    let mut input = InputStack::new(super::WorldInput::from_content(content));

    let error = input
        .next_traced_token(&mut stores)
        .expect_err("invalid UTF-8 must be rejected");
    let rendered = ProvenanceResolver::new(&stores)
        .render_diagnostic_site("invalid UTF-8", error.diagnostic_site());
    assert!(rendered.contains("invalid.tex:1:2"), "{rendered}");
    let LexError::InvalidUtf8 { context, .. } = error else {
        panic!("expected invalid UTF-8 error, got {error:?}");
    };
    assert_eq!(context.byte_range(), 1..2);
    assert_eq!(context.line(), 1);
    assert_eq!(context.column(), 1);
}

#[test]
fn endline_derived_tokens_have_inserted_origins() {
    let mut stores = Universe::new();
    stores.set_int_param(IntParam::END_LINE_CHAR, 13);
    let mut lexer = Lexer::new(MemoryInput::new("a\n\n"));

    let first = lexer
        .next_traced_token(&mut stores)
        .expect("source token")
        .expect("source token");
    assert_eq!(first.token(), Some(char_token('a', Catcode::Letter)));
    assert_source_origin(&stores, first.origin(), 0, 1, 0);

    let space = lexer
        .next_traced_token(&mut stores)
        .expect("endline space")
        .expect("endline space");
    assert_eq!(space.token(), Some(char_token(' ', Catcode::Space)));
    let space_parent = assert_inserted_origin(&stores, space.origin(), InsertedOriginKind::EndLine);
    assert_source_origin(&stores, space_parent, 1, 1, 1);

    let par = lexer
        .next_traced_token(&mut stores)
        .expect("paragraph token")
        .expect("paragraph token");
    assert_eq!(par.token(), Some(cs_token(&mut stores, "par")));
    let par_parent = assert_inserted_origin(&stores, par.origin(), InsertedOriginKind::Paragraph);
    assert_source_origin(&stores, par_parent, 2, 2, 0);
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
                macro_arguments,
                ..
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
fn replay_markers_distinguish_frames_with_identical_content() {
    let mut stores = Universe::new();
    let list = stores.intern_token_list(&[char_token('x', Catcode::Letter)]);
    let mut input = InputStack::new(MemoryInput::new("a"));
    let outer = input.push_token_list(list, TokenListReplayKind::Inserted);
    let inner = input.push_token_list(list, TokenListReplayKind::Inserted);

    assert_ne!(outer, inner);
    assert!(input.contains_token_list_replay_marker(outer));
    assert!(input.contains_token_list_replay_marker(inner));
    assert_eq!(
        input.next_token(&mut stores).expect("inner replay"),
        Some(char_token('x', Catcode::Letter))
    );
    assert_eq!(
        input.next_token(&mut stores).expect("outer replay"),
        Some(char_token('x', Catcode::Letter))
    );
    assert!(!input.contains_token_list_replay_marker(inner));
    assert!(input.contains_token_list_replay_marker(outer));
}

#[test]
fn exhausted_nested_replays_finish_before_reading_below_marked_boundary() {
    let mut stores = Universe::new();
    let list = stores.intern_token_list(&[char_token('x', Catcode::Letter)]);
    let mut input = InputStack::new(MemoryInput::new("a"));
    let template = input.push_token_list(list, TokenListReplayKind::Inserted);

    assert_eq!(
        input.next_token(&mut stores).expect("template replay"),
        Some(char_token('x', Catcode::Letter))
    );
    input.push_macro_body(list, MacroArguments::new());
    assert_eq!(
        input.next_token(&mut stores).expect("nested macro replay"),
        Some(char_token('x', Catcode::Letter))
    );

    assert!(input.finish_exhausted_token_list_replay(template, &stores));
    assert_eq!(
        input
            .next_token(&mut stores)
            .expect("source after boundary"),
        Some(char_token('a', Catcode::Letter))
    );
}

#[test]
fn token_list_replay_uses_frame_origin_list_without_changing_semantic_identity() {
    let mut stores = Universe::new();
    let tokens = [
        char_token('x', Catcode::Letter),
        char_token('y', Catcode::Letter),
    ];
    let left_list = stores.intern_token_list(&tokens);
    let right_list = stores.intern_token_list(&tokens);
    assert_eq!(left_list, right_list);

    let left_origin = stores.source_origin(tex_state::SourceId::new(1), 10, 2, 3);
    let right_origin = stores.source_origin(tex_state::SourceId::new(2), 20, 4, 5);
    let left_origins = stores.allocate_origin_list(&[left_origin, left_origin]);
    let right_origins = stores.allocate_origin_list(&[right_origin, right_origin]);

    let mut left = InputStack::new(MemoryInput::new(""));
    left.push_token_list_with_origins(left_list, left_origins, TokenListReplayKind::Inserted);
    let mut right = InputStack::new(MemoryInput::new(""));
    right.push_token_list_with_origins(right_list, right_origins, TokenListReplayKind::Inserted);

    assert_eq!(left.summary(), right.summary());

    let left_replayed = left
        .next_traced_token(&mut stores)
        .expect("token-list replay")
        .expect("token");
    let right_replayed = right
        .next_traced_token(&mut stores)
        .expect("token-list replay")
        .expect("token");

    assert_eq!(left_replayed.token(), Some(tokens[0]));
    assert_eq!(right_replayed.token(), Some(tokens[0]));
    assert_eq!(left_replayed.origin(), left_origin);
    assert_eq!(right_replayed.origin(), right_origin);
}

#[test]
fn macro_body_frame_invocation_origin_does_not_affect_summary_equality() {
    let mut stores = Universe::new();
    let token = char_token('x', Catcode::Letter);
    let token_list = stores.intern_token_list(&[token]);
    let definition_origin = stores.source_origin(tex_state::SourceId::new(1), 10, 2, 3);
    let left_call = stores.source_origin(tex_state::SourceId::new(2), 20, 4, 5);
    let right_call = stores.source_origin(tex_state::SourceId::new(3), 30, 6, 7);
    let origins = stores.allocate_origin_list(&[definition_origin]);
    let params = stores.intern_token_list(&[]);
    let definition = stores.intern_macro(tex_state::macro_store::MacroMeaning::new(
        tex_state::meaning::MeaningFlags::EMPTY,
        params,
        token_list,
    ));
    let left_invocation =
        stores.macro_invocation_origin(definition, left_call, definition_origin, OriginId::UNKNOWN);
    let right_invocation = stores.macro_invocation_origin(
        definition,
        right_call,
        definition_origin,
        OriginId::UNKNOWN,
    );
    let mut left = InputStack::new(MemoryInput::new(""));
    left.push_macro_body_with_origins_and_invocation(
        token_list,
        origins,
        MacroArguments::new(),
        left_invocation,
    );
    let mut right = InputStack::new(MemoryInput::new(""));
    right.push_macro_body_with_origins_and_invocation(
        token_list,
        origins,
        MacroArguments::new(),
        right_invocation,
    );

    assert_eq!(left.summary(), right.summary());
}

#[test]
fn macro_body_replay_without_origin_list_delivers_unknown_origin() {
    let mut stores = Universe::new();
    let token = char_token('x', Catcode::Letter);
    let token_list = stores.intern_token_list(&[token]);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_macro_body_with_origins(token_list, OriginListId::EMPTY, MacroArguments::new());

    let replayed = input
        .next_traced_token(&mut stores)
        .expect("token-list replay")
        .expect("token");

    assert_eq!(replayed.token(), Some(token));
    assert_eq!(replayed.origin(), tex_state::token::OriginId::UNKNOWN);
}

#[test]
fn macro_literal_spans_copy_body_and_argument_provenance_at_matching_offsets() {
    let mut stores = Universe::new();
    let stop = stores.intern("stop");
    let body_tokens = [
        char_token('a', Catcode::Letter),
        char_token('b', Catcode::Other),
        Token::param(1),
        char_token('c', Catcode::Letter),
        Token::Cs(stop.symbol()),
    ];
    let argument_tokens = [
        char_token('x', Catcode::Letter),
        char_token('y', Catcode::Other),
    ];
    let body = stores.intern_token_list(&body_tokens);
    let argument = stores.intern_token_list(&argument_tokens);
    let body_origin = stores.source_origin(tex_state::SourceId::new(1), 10, 1, 1);
    let argument_origin = stores.source_origin(tex_state::SourceId::new(2), 20, 2, 1);
    let body_origins = stores.allocate_origin_list(&[body_origin; 5]);
    let argument_origins = stores.allocate_origin_list(&[argument_origin; 2]);
    let mut arguments = MacroArguments::new();
    arguments.set_traced(1, TracedTokenList::new(argument, argument_origins));

    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_macro_body_with_origins(body, body_origins, arguments);
    let mut tokens = stores.token_list_builder();
    let mut origins = stores.origin_list_builder();

    assert_eq!(
        input.append_macro_literal_span(
            &stores,
            &mut tokens,
            &mut origins,
            LiteralSpanPolicy::ExpandedReplacement,
        ),
        2
    );
    assert_eq!(
        input.append_macro_literal_span(
            &stores,
            &mut tokens,
            &mut origins,
            LiteralSpanPolicy::ExpandedReplacement,
        ),
        2
    );
    assert_eq!(
        input.append_macro_literal_span(
            &stores,
            &mut tokens,
            &mut origins,
            LiteralSpanPolicy::ExpandedReplacement,
        ),
        1
    );
    assert_eq!(
        input.append_macro_literal_span(
            &stores,
            &mut tokens,
            &mut origins,
            LiteralSpanPolicy::ExpandedReplacement,
        ),
        0
    );

    let token_list = stores.finish_token_list(&mut tokens);
    let origin_list = stores.finish_origin_list(&mut origins);
    assert_eq!(
        stores.tokens(token_list),
        [
            body_tokens[0],
            body_tokens[1],
            argument_tokens[0],
            argument_tokens[1],
            body_tokens[3],
        ]
    );
    assert_eq!(
        stores.origin_list(origin_list),
        [
            body_origin,
            body_origin,
            argument_origin,
            argument_origin,
            body_origin,
        ]
    );
}

#[test]
fn macro_literal_span_deopts_for_any_active_alignment_scanner() {
    let mut stores = Universe::new();
    let body = stores.intern_token_list(&[
        char_token('x', Catcode::Letter),
        char_token('&', Catcode::AlignmentTab),
    ]);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_macro_body(body, MacroArguments::new());
    input.begin_alignment();
    let mut tokens = stores.token_list_builder();
    let mut origins = stores.origin_list_builder();

    assert_eq!(
        input.append_macro_literal_span(
            &stores,
            &mut tokens,
            &mut origins,
            LiteralSpanPolicy::ExpandedReplacement,
        ),
        0
    );
    assert_eq!(
        input
            .next_traced_expansion_token(&mut stores)
            .expect("ordinary replay")
            .expect("first token")
            .token(),
        char_token('x', Catcode::Letter)
    );
}

#[cfg(feature = "expansion-stats")]
#[test]
fn expansion_stats_measure_literal_runs_and_segmentation_reuse() {
    let mut stores = Universe::new();
    let body = stores.intern_token_list(&[
        char_token('a', Catcode::Letter),
        char_token('b', Catcode::Letter),
        char_token('c', Catcode::Other),
    ]);
    let mut input = InputStack::new(MemoryInput::new(""));
    let mut tokens = stores.token_list_builder();
    let mut origins = stores.origin_list_builder();

    input.push_macro_body(body, MacroArguments::new());
    assert_eq!(
        input.append_macro_literal_span(
            &stores,
            &mut tokens,
            &mut origins,
            LiteralSpanPolicy::ExpandedReplacement,
        ),
        3
    );
    input.push_macro_body(body, MacroArguments::new());
    assert_eq!(
        input.append_macro_literal_span(
            &stores,
            &mut tokens,
            &mut origins,
            LiteralSpanPolicy::ExpandedReplacement,
        ),
        3
    );

    let stats = input.expansion_stats();
    assert_eq!(stats.literal_spans, 2);
    assert_eq!(stats.literal_tokens, 6);
    assert_eq!(stats.mean_literal_run(), 3.0);
    assert_eq!(stats.segmentation_cache_misses, 1);
    assert_eq!(stats.segmentation_cache_hits, 1);
    assert_eq!(stats.builder_appends, 6);
    assert_eq!(stats.builder_append_timer_samples, 2);
}

#[cfg(feature = "expansion-stats")]
#[test]
fn macro_site_meaning_cache_is_guarded_across_writes_groups_and_rollback() {
    let mut stores = Universe::new();
    let symbol = stores.intern("cached");
    stores.set_meaning(symbol, Meaning::Relax);
    let baseline = stores.snapshot();
    let body = stores.intern_token_list(&[Token::Cs(symbol.symbol())]);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_macro_body(body, MacroArguments::new());

    let token = input
        .next_traced_expansion_token(&mut stores)
        .expect("macro replay")
        .expect("control sequence");
    assert_eq!(token.token(), Token::Cs(symbol.symbol()));
    assert_eq!(
        input.resolve_expansion_meaning(&stores, symbol.symbol()),
        Meaning::Relax
    );
    assert_eq!(
        input.resolve_expansion_meaning(&stores, symbol.symbol()),
        Meaning::Relax
    );

    stores.enter_group();
    stores.set_meaning(symbol, Meaning::Undefined);
    assert_eq!(
        input.resolve_expansion_meaning(&stores, symbol.symbol()),
        Meaning::Undefined
    );
    let _ = stores.leave_group();
    assert_eq!(
        input.resolve_expansion_meaning(&stores, symbol.symbol()),
        Meaning::Relax
    );

    stores.set_meaning(symbol, Meaning::Undefined);
    stores.rollback(&baseline);
    assert_eq!(
        input.resolve_expansion_meaning(&stores, symbol.symbol()),
        Meaning::Relax
    );

    let fork = stores.clone();
    assert_eq!(
        input.resolve_expansion_meaning(&fork, symbol.symbol()),
        Meaning::Relax
    );

    let stats = input.expansion_stats();
    assert_eq!(stats.meaning_cache_hits, 1);
    assert_eq!(stats.meaning_cache_misses, 5);
    assert_eq!(stats.meaning_lookups, 5);
    assert_eq!(stats.frame_step_timer_samples, 1);
    assert_eq!(stats.provenance_timer_samples, 1);
    assert_eq!(stats.classification_meaning_timer_samples, 6);
    assert_eq!(
        stats.attributed_nanos(),
        stats
            .frame_step_nanos
            .saturating_add(stats.provenance_nanos)
            .saturating_add(stats.classification_meaning_nanos)
            .saturating_add(stats.builder_append_nanos)
    );
}

#[test]
fn stale_replay_origin_list_degrades_to_unknown_after_rollback() {
    let mut stores = Universe::new();
    let token = char_token('x', Catcode::Letter);
    let token_list = stores.intern_token_list(&[token]);
    let snapshot = stores.snapshot();
    let origin = stores.source_origin(tex_state::SourceId::new(1), 10, 2, 3);
    let origins = stores.allocate_origin_list(&[origin]);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list_with_origins(token_list, origins, TokenListReplayKind::Inserted);

    stores.rollback(&snapshot);
    let replayed = input
        .next_traced_token(&mut stores)
        .expect("stale diagnostic side table must not abort replay")
        .expect("semantic token list remains live");

    assert_eq!(replayed.token(), Some(token));
    assert_eq!(replayed.origin(), OriginId::UNKNOWN);
}

#[test]
fn nested_popped_invocations_retain_the_complete_parent_chain_for_one_delivery_attempt() {
    let mut stores = Universe::new();
    stores.set_int_param(IntParam::END_LINE_CHAR, -1);
    let empty = stores.intern_token_list(&[]);
    let definition = stores.intern_macro(tex_state::macro_store::MacroMeaning::new(
        tex_state::meaning::MeaningFlags::EMPTY,
        empty,
        empty,
    ));
    let definition_origin = stores.source_origin(tex_state::SourceId::new(1), 0, 1, 1);
    let mut input = InputStack::new(MemoryInput::new(""));
    let mut invocations = Vec::new();
    for offset in 0..64 {
        let call = stores.source_origin(
            tex_state::SourceId::new(2),
            u64::try_from(offset).expect("small test offset"),
            1,
            u32::try_from(offset + 1).expect("small test column"),
        );
        let invocation = stores.macro_invocation_origin(
            definition,
            call,
            definition_origin,
            input.active_macro_invocation(),
        );
        invocations.push(invocation);
        input.push_macro_body_with_origins_and_invocation(
            empty,
            OriginListId::EMPTY,
            MacroArguments::new(),
            invocation,
        );
    }

    let summary = input.summary();
    let mut input = InputStack::from_summary(&summary, |_, _, _| {
        Ok::<_, std::convert::Infallible>(MemoryInput::new(""))
    })
    .expect("input summary should restore");
    assert_eq!(
        input.active_macro_invocation(),
        invocations.last().copied().expect("nested invocation")
    );

    assert!(
        input
            .next_traced_token(&mut stores)
            .expect("empty nested replay should reach EOF")
            .is_none()
    );
    let site = input.diagnostic_site(None, []);
    let mut actual = Vec::new();
    let mut current = site.expansion_head();
    while let Some(origin) = current {
        actual.push(origin);
        let OriginRecord::MacroInvocation(invocation) = stores.origin(origin) else {
            panic!("expansion chain must contain only macro invocation origins");
        };
        current = (invocation.parent_invocation() != OriginId::UNKNOWN)
            .then_some(invocation.parent_invocation());
    }
    assert_eq!(
        actual,
        invocations.iter().rev().copied().collect::<Vec<_>>()
    );

    assert!(
        input
            .next_traced_token(&mut stores)
            .expect("repeated EOF should remain harmless")
            .is_none()
    );
    assert_eq!(input.diagnostic_site(None, []).expansion_head(), None);
}

#[test]
#[should_panic(expected = "token-list replay origin-list length does not match token-list length")]
fn token_list_replay_checks_origin_list_length() {
    let mut stores = Universe::new();
    let token_list = stores.intern_token_list(&[
        char_token('x', Catcode::Letter),
        char_token('y', Catcode::Letter),
    ]);
    let origin = stores.source_origin(tex_state::SourceId::new(1), 10, 2, 3);
    let origins = stores.allocate_origin_list(&[origin]);

    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list_with_origins(token_list, origins, TokenListReplayKind::Inserted);
    let _ = input.next_traced_token(&mut stores);
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
            ..
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
    let [
        InputFrameSummary::Source {
            source_id, source, ..
        },
    ] = summary.frames()
    else {
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
fn source_summary_captures_blank_line_with_endlinechar() {
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

    assert_eq!(source.normalized_line(), "\r");
    assert_eq!(source.line_char_offset(), 0);
    assert_eq!(source.line_byte_offset(), 0);
    assert_eq!(source.line_number(), 1);
    assert!(source.pending().is_empty());
    assert!(source.is_resume_complete());

    assert_eq!(
        input.next_token(&mut stores).expect("blank line token"),
        Some(cs_token(&mut stores, "par"))
    );
}

#[test]
fn condition_frames_round_trip_through_input_summary() {
    let mut stores = Universe::new();
    stores.set_int_param(IntParam::END_LINE_CHAR, 13);
    let mut input = InputStack::new(MemoryInput::new("ab"));
    let condition = ConditionFrameSummary::new_ifcase(condition_context(), false)
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
            InputFrameSummary::Condition { condition: frame, .. },
        ] if frame.kind() == ConditionKind::IfCase
            && frame.limb() == ConditionLimb::Or
            && !frame.evaluating()
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
            InputFrameSummary::Condition { condition: frame, .. },
        ] if source.column() == 1 && *frame == condition
    ));
}

#[test]
fn frozen_alignment_token_survives_input_summary_restore() {
    let mut stores = Universe::new();
    let tokens = stores.intern_token_list(&[stores.frozen_end_template_token()]);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_token_list(tokens, TokenListReplayKind::Inserted);
    let summary = input.summary();
    let mut restored =
        InputStack::from_summary(&summary, |_, _, _| Ok::<_, ()>(MemoryInput::new("")))
            .expect("restore frozen-token input stack");

    assert_eq!(
        restored.next_token(&mut stores).expect("restored token"),
        Some(stores.frozen_end_template_token())
    );
}

#[test]
fn open_condition_survives_checkpoint_rollback_resume_summary() {
    let mut stores = Universe::new();
    stores.set_int_param(IntParam::END_LINE_CHAR, 13);
    let mut input = InputStack::new(MemoryInput::new("xy"));
    let frame_token =
        input.push_condition(ConditionFrameSummary::new_if(condition_context(), true));

    assert_eq!(
        input.next_token(&mut stores).expect("source token"),
        Some(char_token('x', Catcode::Letter))
    );
    let checkpoint = stores.snapshot();
    let resume_summary = input.summary();

    let updated = ConditionFrameSummary::new_if(condition_context(), true).with_else_limb(false);
    assert_eq!(
        input.current_condition(),
        Some(ConditionFrameSummary::new_if(condition_context(), true))
    );
    assert_eq!(
        input.update_condition(frame_token, updated),
        Some(ConditionFrameSummary::new_if(condition_context(), true))
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
            InputFrameSummary::Condition { condition: frame, .. },
        ] if source.column() == 1
            && frame.kind() == ConditionKind::If
            && frame.limb() == ConditionLimb::If
            && !frame.evaluating()
            && frame.current_limb_taken()
            && frame.any_limb_taken()
            && frame.ifcase_or_count() == 0
            && frame.skip_nesting() == 0
    ));
}

#[test]
fn condition_identity_targets_frame_below_nested_condition_and_survives_summary() {
    let mut input = InputStack::new(MemoryInput::new(""));
    let context = condition_context();
    let outer = input.push_condition(ConditionFrameSummary::evaluating_if(context));
    let nested = input.push_condition(ConditionFrameSummary::new_if(context, true));

    assert_eq!(input.current_condition_token(), Some(nested));
    assert_eq!(
        input.update_condition(outer, ConditionFrameSummary::new_if(context, false)),
        Some(ConditionFrameSummary::evaluating_if(context))
    );
    assert_eq!(
        input.current_condition(),
        Some(ConditionFrameSummary::new_if(context, true))
    );

    let summary = input.summary();
    let mut restored =
        InputStack::from_summary(&summary, |_, _, _| Ok::<_, ()>(MemoryInput::new("")))
            .expect("condition summary restores");
    assert_eq!(restored.summary(), summary);
    assert_eq!(restored.current_condition_token(), Some(nested));
    assert_eq!(
        restored.update_condition(outer, ConditionFrameSummary::new_if(context, true)),
        Some(ConditionFrameSummary::new_if(context, false))
    );
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
    stores
        .world_mut()
        .set_memory_file("font.tfm", b"auxiliary metrics".to_vec())
        .expect("seed auxiliary input");
    let main = stores.world_mut().read_file("main.tex").expect("read main");
    stores
        .world_mut()
        .read_file("font.tfm")
        .expect("read auxiliary input");
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
    let summary = input.publication_summary(&mut stores);
    stores.set_input_summary(summary);
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
    let mut restored = InputStack::from_summary(&summary, |_source_id, input_record, source| {
        let content = stores
            .world()
            .recorded_input_content(
                input_record.expect("world input frame retains its input record"),
            )
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
    Token::Cs(stores.intern(name).symbol())
}

fn assert_source_origin(
    stores: &Universe,
    origin: tex_state::token::OriginId,
    byte_offset: u64,
    line: u32,
    column: u32,
) {
    assert_source_origin_for(
        stores,
        origin,
        tex_state::SourceId::new(0),
        byte_offset,
        line,
        column,
    );
}

fn assert_source_origin_for(
    stores: &Universe,
    origin: tex_state::token::OriginId,
    source_id: tex_state::SourceId,
    byte_offset: u64,
    line: u32,
    column: u32,
) {
    match stores.origin(origin) {
        OriginRecord::Source(source) => {
            assert_eq!(source.source(), source_id);
            assert_eq!(source.byte_offset(), byte_offset);
            assert_eq!(source.line(), line);
            assert_eq!(source.column(), column);
        }
        OriginRecord::SourceSpan(span) => {
            assert_eq!(
                span.lo(),
                stores
                    .source_position(source_id, byte_offset)
                    .expect("direct source position must stay live")
            );
        }
        other => panic!("expected source origin, got {other:?}"),
    }
}

fn assert_inserted_origin(
    stores: &Universe,
    origin: tex_state::token::OriginId,
    kind: InsertedOriginKind,
) -> tex_state::token::OriginId {
    let OriginRecord::Inserted(inserted) = stores.origin(origin) else {
        panic!("expected inserted origin, got {:?}", stores.origin(origin));
    };
    assert_eq!(inserted.kind(), kind);
    inserted.parent()
}
