use super::{
    AlignmentTerminator, AlignmentTokenDelivery, ConditionFrameSummary, ConditionKind,
    ConditionLimb, ImmutableSourceKind, InputFrame, InputFrameSummary, InputSource, InputStack,
    LayoutCursor, LayoutCursorError, LexError, Lexer, LexerState, LineEvent, LineReader,
    LiteralSpanPolicy, MACRO_ARGUMENT_SLOTS, MacroArgumentRange, MacroArguments, MemoryInput,
    PhysicalLine, StableSourceSpanId, TokenListReplayKind, load_next_line,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tex_state::env::banks::IntParam;
use tex_state::ids::{OriginListId, TokenListId};
use tex_state::provenance::{InsertedOriginKind, OriginRecord};
use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};
use tex_state::{
    ContentHash, EditorLayout, ExpansionState, FragmentStore, LayoutGeneration, Piece,
    ProvenanceResolver, Universe,
};

mod input_lines;

#[test]
fn memory_input_descriptors_preserve_only_explicit_logical_paths() {
    let named = MemoryInput::new("root").with_logical_path("editor/root.tex");
    let anonymous = MemoryInput::new("generated");

    let tex_state::source_map::SourceDescriptor::Generated(named) =
        named.source_descriptor().expect("named descriptor")
    else {
        panic!("memory input must be generated")
    };
    let tex_state::source_map::SourceDescriptor::Generated(anonymous) =
        anonymous.source_descriptor().expect("anonymous descriptor")
    else {
        panic!("memory input must be generated")
    };

    assert_eq!(named.logical_path(), Some("editor/root.tex"));
    assert_eq!(anonymous.logical_path(), None);
}

#[test]
fn executor_step_snapshot_restores_complete_live_input_without_host_lookup() {
    #[derive(Clone, Debug)]
    struct CountingInput {
        inner: MemoryInput,
        reads: Arc<AtomicUsize>,
    }

    impl InputSource for CountingInput {
        fn clone_input_source(&self) -> Box<dyn InputSource> {
            Box::new(self.clone())
        }

        fn read_line(&mut self) -> Result<Option<PhysicalLine>, super::InputSourceError> {
            self.reads.fetch_add(1, Ordering::Relaxed);
            self.inner.read_line()
        }

        fn source_descriptor(&self) -> Option<tex_state::source_map::SourceDescriptor> {
            self.inner.source_descriptor()
        }
    }

    let reads = Arc::new(AtomicUsize::new(0));
    let mut stores = Universe::new();
    stores.set_int_param(IntParam::END_LINE_CHAR, 13);
    let mut input = InputStack::new(CountingInput {
        inner: MemoryInput::new("a\nb"),
        reads: Arc::clone(&reads),
    });
    assert_eq!(
        input.next_token(&mut stores).expect("first source token"),
        Some(char_token('a', Catcode::Letter))
    );

    let macro_token = char_token('m', Catcode::Letter);
    let macro_body = stores.intern_token_list(&[macro_token]);
    let macro_origin = stores.source_origin(tex_state::SourceId::new(7), 11, 2, 3);
    let macro_origins = stores.allocate_origin_list(&[macro_origin]);
    input.push_macro_body_with_origins_and_invocation(
        macro_body,
        macro_origins,
        MacroArguments::new(),
        macro_origin,
    );
    let transient_token = char_token('t', Catcode::Other);
    let transient_marker = input.push_transient_tokens(
        vec![TracedTokenWord::pack(transient_token, macro_origin)],
        TokenListReplayKind::Inserted,
    );
    let condition = input.push_condition(ConditionFrameSummary::new_if(condition_context(), true));
    input.begin_alignment();
    input.set_alignment_state(0);
    input.begin_alignment_cell(Some(transient_marker), macro_body, 7);
    input.literal_span_cache.insert(
        (macro_body, LiteralSpanPolicy::ExpandedReplacement),
        Arc::from([(0, 1)]),
    );
    input.recycle_transient_token_buffer(Vec::with_capacity(8));
    input.unicode_superscript_notation = false;
    input.utf8_input_as_bytes = true;
    input.recently_popped_invocation = Some(macro_origin);
    #[cfg(feature = "profiling-stats")]
    {
        input.expansion_stats.meaning_lookups = 17;
        input.expansion_stats.frame_step_timer_events = 23;
    }

    let expected = format!("{input:#?}");
    let summary_before = input.summary();
    let reads_before = reads.load(Ordering::Relaxed);
    let snapshot = input.snapshot();
    assert_eq!(reads.load(Ordering::Relaxed), reads_before);

    input.frames = super::StableFrames::new();
    input.source_frame_count = 0;
    input.token_frame_indices.clear();
    input.condition_frame_indices.clear();
    input.next_source_id = u32::MAX;
    input.unicode_superscript_notation = true;
    input.utf8_input_as_bytes = false;
    input.last_source_frame = None;
    input.next_replay_marker = u64::MAX;
    input.next_condition_token = u64::MAX;
    input.alignment_inputs.clear();
    input.literal_span_cache.clear();
    input.transient_buffer_pool.clear();
    input.active_macro_invocation = OriginId::UNKNOWN;
    input.recently_popped_invocation = None;
    #[cfg(feature = "profiling-stats")]
    {
        input.expansion_stats = super::ExpansionStats::default();
    }

    input.rollback(snapshot);
    assert_eq!(reads.load(Ordering::Relaxed), reads_before);
    assert_eq!(format!("{input:#?}"), expected);
    assert_eq!(input.summary(), summary_before);
    assert_eq!(input.current_condition_token(), Some(condition));
    assert!(input.contains_token_list_replay_marker(transient_marker));
    assert!(input.alignment_state_is(0));
    assert_eq!(input.active_macro_invocation(), macro_origin);
    assert_eq!(input.transient_buffer_pool.len(), 1);
    assert_eq!(input.literal_span_cache.len(), 1);
    #[cfg(feature = "profiling-stats")]
    {
        assert_eq!(input.expansion_stats().meaning_lookups, 17);
        assert_eq!(input.expansion_stats.frame_step_timer_events, 23);
    }

    let transient = input
        .next_traced_token(&mut stores)
        .expect("restored transient replay")
        .expect("transient token");
    assert_eq!(transient.unpack(), Some((transient_token, macro_origin)));
    let macro_replay = input
        .next_traced_token(&mut stores)
        .expect("restored macro replay")
        .expect("macro token");
    assert_eq!(macro_replay.unpack(), Some((macro_token, macro_origin)));
    assert_eq!(
        input.next_token(&mut stores).expect("restored line ending"),
        Some(char_token(' ', Catcode::Space))
    );
    assert_eq!(
        input.next_token(&mut stores).expect("restored next line"),
        Some(char_token('b', Catcode::Letter))
    );
    assert_eq!(reads.load(Ordering::Relaxed), reads_before + 1);
}

#[test]
fn macro_argument_replay_and_snapshots_share_the_matched_buffer() {
    let mut stores = Universe::new();
    let body = stores.intern_token_list(&[Token::param(1)]);
    let argument_tokens = [
        char_token('a', Catcode::Letter),
        char_token('b', Catcode::Letter),
        char_token('c', Catcode::Letter),
    ];
    let argument_words = argument_tokens
        .into_iter()
        .map(|token| TracedTokenWord::pack(token, OriginId::UNKNOWN))
        .collect::<Vec<_>>();
    let mut ranges = [None; MACRO_ARGUMENT_SLOTS];
    ranges[0] = Some(MacroArgumentRange::new(0, argument_words.len()));
    let arguments = MacroArguments::from_parts(argument_words, ranges);
    let shared_arguments = Arc::clone(
        arguments
            .tokens
            .as_ref()
            .expect("nonempty arguments have shared storage"),
    );

    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_macro_body(body, arguments);
    assert_eq!(
        input.next_token(&mut stores).expect("first argument token"),
        Some(argument_tokens[0])
    );

    let replay_tokens = input
        .frames
        .iter()
        .find_map(|frame| match frame {
            InputFrame::TokenList(frame)
                if matches!(frame.replay_kind, TokenListReplayKind::MacroArgument) =>
            {
                let super::ReplayPayload::MacroArgument { tokens, .. } = &frame.payload else {
                    panic!("macro argument replay must use shared storage")
                };
                Some(tokens)
            }
            _ => None,
        })
        .expect("live argument replay frame");
    assert!(Arc::ptr_eq(replay_tokens, &shared_arguments));

    let snapshot = input.snapshot();
    let snapshot_tokens = snapshot
        .0
        .frames
        .iter()
        .find_map(|frame| match frame {
            InputFrame::TokenList(frame)
                if matches!(frame.replay_kind, TokenListReplayKind::MacroArgument) =>
            {
                let super::ReplayPayload::MacroArgument { tokens, .. } = &frame.payload else {
                    panic!("snapshotted argument replay must use shared storage")
                };
                Some(tokens)
            }
            _ => None,
        })
        .expect("snapshotted argument replay frame");
    assert!(Arc::ptr_eq(snapshot_tokens, &shared_arguments));

    assert_eq!(
        input
            .next_token(&mut stores)
            .expect("second argument token"),
        Some(argument_tokens[1])
    );
    input.rollback(snapshot);
    assert_eq!(
        input
            .next_token(&mut stores)
            .expect("restored second token"),
        Some(argument_tokens[1])
    );
}

#[test]
fn macro_argument_replay_does_not_recursively_substitute_parameter_tokens() {
    let mut stores = Universe::new();
    let body = stores.intern_token_list(&[
        char_token('a', Catcode::Letter),
        Token::param(1),
        char_token('b', Catcode::Letter),
    ]);
    let parameter = TracedTokenWord::pack(Token::param(1), OriginId::UNKNOWN);
    let mut ranges = [None; MACRO_ARGUMENT_SLOTS];
    ranges[0] = Some(MacroArgumentRange::new(0, 1));
    let arguments = MacroArguments::from_parts(vec![parameter], ranges);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_macro_body(body, arguments);

    assert_eq!(
        [
            input.next_token(&mut stores).expect("body prefix"),
            input
                .next_token(&mut stores)
                .expect("literal argument token"),
            input.next_token(&mut stores).expect("body suffix"),
            input.next_token(&mut stores).expect("exhausted replay"),
        ],
        [
            Some(char_token('a', Catcode::Letter)),
            Some(Token::param(1)),
            Some(char_token('b', Catcode::Letter)),
            None,
        ]
    );
}

#[test]
fn nested_token_list_resolves_current_macro_parameter() {
    let mut stores = Universe::new();
    let body_token = char_token('b', Catcode::Letter);
    let argument_token = char_token('a', Catcode::Letter);
    let body = stores.intern_token_list(&[body_token]);
    let nested = stores.intern_token_list(&[Token::param(1)]);
    let argument_words = vec![TracedTokenWord::pack(argument_token, OriginId::UNKNOWN)];
    let mut ranges = [None; MACRO_ARGUMENT_SLOTS];
    ranges[0] = Some(MacroArgumentRange::new(0, 1));
    let arguments = MacroArguments::from_parts(argument_words, ranges);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_macro_body(body, arguments);
    input.push_token_list(nested, TokenListReplayKind::Inserted);

    assert_eq!(
        input
            .next_token(&mut stores)
            .expect("nested parameter replay"),
        Some(argument_token)
    );
    assert_eq!(
        input.next_token(&mut stores).expect("owning macro body"),
        Some(body_token)
    );
}

#[test]
fn nested_token_list_does_not_reach_past_current_macro_arguments() {
    let mut stores = Universe::new();
    let outer_body = stores.intern_token_list(&[char_token('o', Catcode::Letter)]);
    let inner_body = stores.intern_token_list(&[char_token('i', Catcode::Letter)]);
    let nested = stores.intern_token_list(&[Token::param(1)]);
    let outer_argument = TracedTokenWord::pack(char_token('a', Catcode::Letter), OriginId::UNKNOWN);
    let mut ranges = [None; MACRO_ARGUMENT_SLOTS];
    ranges[0] = Some(MacroArgumentRange::new(0, 1));
    let outer_arguments = MacroArguments::from_parts(vec![outer_argument], ranges);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_macro_body(outer_body, outer_arguments);
    input.push_macro_body(inner_body, MacroArguments::new());
    input.push_token_list(nested, TokenListReplayKind::Inserted);

    assert_eq!(
        input.next_token(&mut stores).expect("nested token replay"),
        Some(Token::param(1)),
        "the innermost macro's parameter frame must shadow older invocations"
    );
}

#[test]
fn nested_unexpanded_list_preserves_parameter_token() {
    let mut stores = Universe::new();
    let body = stores.intern_token_list(&[char_token('b', Catcode::Letter)]);
    let unexpanded = stores.intern_token_list(&[Token::param(1)]);
    let argument = TracedTokenWord::pack(char_token('a', Catcode::Letter), OriginId::UNKNOWN);
    let mut ranges = [None; MACRO_ARGUMENT_SLOTS];
    ranges[0] = Some(MacroArgumentRange::new(0, 1));
    let arguments = MacroArguments::from_parts(vec![argument], ranges);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_macro_body(body, arguments);
    input.push_token_list(unexpanded, TokenListReplayKind::Unexpanded);

    assert_eq!(
        input.next_token(&mut stores).expect("unexpanded replay"),
        Some(Token::param(1)),
        "e-TeX unexpanded output is copied rather than read through get_next"
    );
}

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
fn recovery_inserted_left_brace_updates_active_alignment_state() {
    let mut input = InputStack::new(MemoryInput::new(""));
    input.begin_alignment();
    input.set_alignment_state(0);

    input.account_inserted_alignment_left_brace();

    assert!(input.alignment_state_is(1));
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
    #[derive(Clone, Debug)]
    struct FailingInput(Option<tex_state::WorldError>);

    impl InputSource for FailingInput {
        fn clone_input_source(&self) -> Box<dyn InputSource> {
            Box::new(self.clone())
        }

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
fn layout_cursor_hands_each_physical_line_its_fragment_registration() {
    let (fragments, layout, registrations) = three_line_fragment_layout();
    let cursor = LayoutCursor::new(&layout, &fragments).expect("line-aligned layout freezes");
    let mut stores = Universe::new();
    stores.set_int_param(IntParam::END_LINE_CHAR, -1);
    let mut input = InputStack::new(MemoryInput::new("aa\nbb\ncc"));
    assert_eq!(
        input.install_root_layout_cursor(cursor),
        Some(tex_state::SourceId::new(0))
    );

    let expected = [
        registrations[0]
            .direct_origin(0, 1)
            .expect("first fragment origin"),
        registrations[0]
            .direct_origin(1, 2)
            .expect("first fragment origin"),
        registrations[1]
            .direct_origin(0, 1)
            .expect("second fragment origin"),
        registrations[1]
            .direct_origin(1, 2)
            .expect("second fragment origin"),
        registrations[2]
            .direct_origin(0, 1)
            .expect("third fragment origin"),
        registrations[2]
            .direct_origin(1, 2)
            .expect("third fragment origin"),
    ];
    for expected_origin in expected {
        let token = input
            .next_traced_token(&mut stores)
            .expect("layout-backed tokenization succeeds")
            .expect("source character is delivered");
        assert_eq!(token.origin(), expected_origin);
    }
    assert!(
        input
            .next_traced_token(&mut stores)
            .expect("layout-backed EOF succeeds")
            .is_none()
    );
}

#[test]
fn direct_root_delivery_exposes_piece_identity_without_origin_identity() {
    let (fragments, layout, _) = three_line_fragment_layout();
    let expected = fragments
        .root_span_id(&layout.pieces()[0], 0..1)
        .expect("expected root identity");
    let mut input = InputStack::new(MemoryInput::new("aa\nbb\ncc"));
    input.install_root_layout_cursor(
        LayoutCursor::new(&layout, &fragments).expect("layout cursor freezes"),
    );
    let mut stores = Universe::new();
    stores.set_int_param(IntParam::END_LINE_CHAR, -1);

    let token = input
        .next_traced_token(&mut stores)
        .expect("delivery succeeds")
        .expect("source token");
    let delivery = input
        .take_direct_source_delivery(token)
        .expect("direct delivery proof");

    assert_eq!(delivery.root_span_id(&fragments), Some(expected));
}

#[test]
fn root_cursor_anchor_does_not_refill_underlying_source_during_token_replay() {
    let (fragments, layout, _) = three_line_fragment_layout();
    let expected = fragments
        .root_span_id(&layout.pieces()[0], 2..2)
        .expect("end of first physical line has a stable cursor");
    let mut input = InputStack::new(MemoryInput::new("aa\nbb\ncc"));
    input.install_root_layout_cursor(
        LayoutCursor::new(&layout, &fragments).expect("layout cursor freezes"),
    );
    let mut stores = Universe::new();
    stores
        .install_editor_fragments(&fragments, &layout)
        .expect("editor fragments install");
    stores.set_int_param(IntParam::END_LINE_CHAR, -1);
    for expected_token in ['a', 'a'] {
        assert_eq!(
            input
                .next_traced_token(&mut stores)
                .expect("first line tokenizes")
                .and_then(TracedTokenWord::token),
            Some(char_token(expected_token, Catcode::Letter))
        );
    }
    let replay = stores.intern_token_list(&[char_token('x', Catcode::Letter)]);
    input.push_token_list(replay, TokenListReplayKind::Inserted);
    let before = input.summary();

    assert_eq!(input.root_source_cursor_anchor(&mut stores), Some(expected));
    assert_eq!(
        input.summary(),
        before,
        "anchor observation must not load line 2"
    );
}

#[test]
fn immutable_source_delivery_identity_uses_content_not_runtime_record() {
    let mut stores = Universe::new();
    stores.set_int_param(IntParam::END_LINE_CHAR, -1);
    stores
        .world_mut()
        .set_memory_file("included.tex", b"x".to_vec())
        .expect("seed include");
    let content = stores
        .world_mut()
        .read_file("included.tex")
        .expect("open include");
    let expected = content.hash();
    let mut included = InputStack::new(super::WorldInput::from_content(content));
    let token = included
        .next_traced_token(&mut stores)
        .expect("include delivery")
        .expect("include token");
    let delivery = included
        .take_direct_source_delivery(token)
        .expect("include proof")
        .stable_id(&stores, &FragmentStore::new())
        .expect("stable include identity");
    assert_eq!(
        delivery.span(),
        StableSourceSpanId::Immutable {
            kind: ImmutableSourceKind::Included,
            content: expected,
            start: 0,
            end: 1,
        }
    );

    let mut generated_stores = Universe::new();
    generated_stores.set_int_param(IntParam::END_LINE_CHAR, -1);
    let mut generated = InputStack::new(MemoryInput::new("x"));
    let token = generated
        .next_traced_token(&mut generated_stores)
        .expect("generated delivery")
        .expect("generated token");
    let delivery = generated
        .take_direct_source_delivery(token)
        .expect("generated proof")
        .stable_id(&generated_stores, &FragmentStore::new())
        .expect("stable generated identity");
    assert_eq!(
        delivery.span(),
        StableSourceSpanId::Immutable {
            kind: ImmutableSourceKind::Generated,
            content: ContentHash::from_bytes(b"x"),
            start: 0,
            end: 1,
        }
    );
}

#[test]
fn physical_line_and_normalization_identities_cover_exact_inputs() {
    let lf = PhysicalLine::new("a  ".to_owned(), 0, 4);
    let shifted_lf = PhysicalLine::new("a  ".to_owned(), 100, 104);
    let crlf = PhysicalLine::new("a  ".to_owned(), 0, 5);
    let missing = PhysicalLine::new("a  ".to_owned(), 0, 3);

    assert_eq!(lf.identity(), shifted_lf.identity());
    assert_ne!(lf.identity(), crlf.identity());
    assert_ne!(lf.identity(), missing.identity());

    let ordinary = lf.normalized_identity(13, false);
    let scan = lf.normalized_identity(13, true);
    let changed_endline = lf.normalized_identity(-1, false);
    assert_ne!(ordinary.key(), scan.key());
    assert_ne!(ordinary.key(), changed_endline.key());
    assert_ne!(ordinary.content(), changed_endline.content());
    assert_eq!(
        ordinary.content(),
        crlf.normalized_identity(13, false).content()
    );
}

#[test]
fn line_reader_reuses_only_complete_normalization_keys() {
    let mut stores = Universe::new();
    stores.set_int_param(IntParam::END_LINE_CHAR, 13);
    let mut reader = LineReader::new(MemoryInput::new("same\nsame\nsame\n"));

    assert_eq!(
        reader.next_event(&stores).expect("first line"),
        Some(LineEvent::Text("same\r".into()))
    );
    assert_eq!(reader.normalization_cache().hits(), 0);
    assert_eq!(
        reader.next_event(&stores).expect("second line"),
        Some(LineEvent::Text("same\r".into()))
    );
    assert_eq!(reader.normalization_cache().hits(), 1);

    stores.set_int_param(IntParam::END_LINE_CHAR, -1);
    assert_eq!(
        reader.next_event(&stores).expect("third line"),
        Some(LineEvent::Text("same".into()))
    );
    assert_eq!(reader.normalization_cache().hits(), 1);
    assert_eq!(reader.normalization_cache().len(), 2);
    assert!(reader.normalization_cache().retained_bytes() > 0);
}

#[test]
fn immutable_world_lines_bypass_source_local_normalization_cache() {
    let mut stores = Universe::new();
    stores.set_int_param(IntParam::END_LINE_CHAR, 13);
    stores
        .world_mut()
        .set_memory_file("repeat.tex", b"same\nsame\n".to_vec())
        .expect("seed immutable input");
    let content = stores
        .world_mut()
        .read_file("repeat.tex")
        .expect("open immutable input");
    let mut reader = LineReader::new(super::WorldInput::from_content(content));

    assert_eq!(
        reader.next_event(&stores).expect("first line"),
        Some(LineEvent::Text("same\r".into()))
    );
    assert_eq!(
        reader.next_event(&stores).expect("second line"),
        Some(LineEvent::Text("same\r".into()))
    );
    assert_eq!(reader.normalization_cache().hits(), 0);
    assert!(reader.normalization_cache().is_empty());
}

#[test]
fn layout_cursor_scalar_crossing_direct_boundary_uses_fragment_span() {
    let mut fragments = FragmentStore::new();
    let (fragment, registration) = fragments
        .testing_append_at(Arc::from(&b"ab"[..]), 1, 0x7fff_fffe)
        .expect("boundary-crossing fragment appends");
    let layout = EditorLayout::new(
        "root.tex",
        LayoutGeneration::new(1),
        vec![Piece::new(fragment, 0, 2)],
        &fragments,
    )
    .expect("layout is valid");
    let mut input = InputStack::new(MemoryInput::new("ab"));
    let stable_source = input
        .install_root_layout_cursor(
            LayoutCursor::new(&layout, &fragments).expect("layout cursor freezes"),
        )
        .expect("root source exists");
    let mut stores = Universe::new();
    stores.set_int_param(IntParam::END_LINE_CHAR, -1);

    let direct = input
        .next_traced_token(&mut stores)
        .expect("direct token succeeds")
        .expect("direct token");
    assert_eq!(
        direct.origin(),
        registration
            .direct_origin(0, 1)
            .expect("last direct origin")
    );

    let wide = input
        .next_traced_token(&mut stores)
        .expect("wide token succeeds")
        .expect("wide token");
    let OriginRecord::SourceSpan(span) = stores.origin(wide.origin()) else {
        panic!("wide fragment scalar must use an arena source span");
    };
    assert_eq!(span, registration.span(1, 2).expect("wide fragment span"));
    assert!(
        stores.source_position(stable_source, 1).is_err(),
        "the stable editor source id must remain absent from the source map"
    );
}

#[test]
fn layout_cursor_preserves_transformed_spans_and_synthetic_anchors() {
    let mut fragments = FragmentStore::new();
    let (first_id, first) = fragments
        .append(Arc::from(&b"^^61\n"[..]), 1)
        .expect("first fragment appends");
    let (second_id, second) = fragments
        .append(Arc::from(&b"^^62"[..]), 1)
        .expect("second fragment appends");
    let layout = EditorLayout::new(
        "root.tex",
        LayoutGeneration::new(1),
        vec![Piece::new(first_id, 0, 5), Piece::new(second_id, 0, 4)],
        &fragments,
    )
    .expect("layout is valid");
    let mut input = InputStack::new(MemoryInput::new("^^61\n^^62"));
    input.install_root_layout_cursor(
        LayoutCursor::new(&layout, &fragments).expect("line-aligned layout freezes"),
    );
    let mut stores = Universe::new();
    stores.set_int_param(IntParam::END_LINE_CHAR, 13);

    let transformed = input
        .next_traced_token(&mut stores)
        .expect("first transform succeeds")
        .expect("first transformed token");
    let OriginRecord::SourceSpan(first_span) = stores.origin(transformed.origin()) else {
        panic!("transformed spelling must retain an exact span");
    };
    assert_eq!(first_span, first.span(0, 4).expect("first spelling span"));

    let endline = input
        .next_traced_token(&mut stores)
        .expect("synthetic endline succeeds")
        .expect("synthetic endline token");
    let anchor = assert_inserted_origin(&stores, endline.origin(), InsertedOriginKind::EndLine);
    let OriginRecord::SourceSpan(anchor_span) = stores.origin(anchor) else {
        panic!("synthetic endline parent must retain its fragment anchor");
    };
    assert_eq!(anchor_span, first.span(4, 4).expect("line anchor span"));

    let transformed = input
        .next_traced_token(&mut stores)
        .expect("second transform succeeds")
        .expect("second transformed token");
    let OriginRecord::SourceSpan(second_span) = stores.origin(transformed.origin()) else {
        panic!("transformed spelling must retain an exact span");
    };
    assert_eq!(
        second_span,
        second.span(0, 4).expect("second spelling span")
    );
}

#[test]
fn restored_summary_reinstalls_cursor_without_changing_root_source_id() {
    let (fragments, layout, registrations) = three_line_fragment_layout();
    let cursor = LayoutCursor::new(&layout, &fragments).expect("line-aligned layout freezes");
    let mut stores = Universe::new();
    stores.set_int_param(IntParam::END_LINE_CHAR, -1);
    let source = "aa\nbb\ncc";
    let mut input = InputStack::new(MemoryInput::new(source));
    let stable_source = input
        .install_root_layout_cursor(cursor.clone())
        .expect("root source exists");
    input
        .next_traced_token(&mut stores)
        .expect("first token succeeds")
        .expect("first token");
    let summary = input.summary();

    let mut restored = InputStack::from_summary(&summary, |_, _, source_summary| {
        Ok::<_, ()>(MemoryInput::from_offset(
            source,
            source_summary.next_source_offset(),
        ))
    })
    .expect("source summary restores");
    assert_eq!(
        restored.install_root_layout_cursor(cursor),
        Some(stable_source)
    );

    let second = restored
        .next_traced_token(&mut stores)
        .expect("restored line succeeds")
        .expect("second first-line token");
    assert_eq!(
        second.origin(),
        registrations[0]
            .direct_origin(1, 2)
            .expect("first fragment origin")
    );
    let third = restored
        .next_traced_token(&mut stores)
        .expect("piece-boundary line succeeds")
        .expect("first second-line token");
    assert_eq!(
        third.origin(),
        registrations[1]
            .direct_origin(0, 1)
            .expect("second fragment origin")
    );
}

#[test]
fn layout_cursor_rejects_piece_boundaries_inside_physical_lines() {
    let mut fragments = FragmentStore::new();
    let (first, _) = fragments
        .append(Arc::from(&b"ab"[..]), 1)
        .expect("first fragment appends");
    let (second, _) = fragments
        .append(Arc::from(&b"cd"[..]), 1)
        .expect("second fragment appends");
    let layout = EditorLayout::new(
        "root.tex",
        LayoutGeneration::new(1),
        vec![Piece::new(first, 0, 2), Piece::new(second, 0, 2)],
        &fragments,
    )
    .expect("piece ranges are structurally valid");
    assert_eq!(
        LayoutCursor::new(&layout, &fragments).expect_err("mid-line boundary must be rejected"),
        LayoutCursorError::PieceBoundaryInsideLine
    );
}

fn three_line_fragment_layout() -> (
    FragmentStore,
    EditorLayout,
    [tex_state::source_map::RegisteredSource; 3],
) {
    let mut fragments = FragmentStore::new();
    let (first_id, first) = fragments
        .append(Arc::from(&b"aa\n"[..]), 1)
        .expect("first fragment appends");
    let (second_id, second) = fragments
        .append(Arc::from(&b"bb\n"[..]), 1)
        .expect("second fragment appends");
    let (third_id, third) = fragments
        .append(Arc::from(&b"cc"[..]), 1)
        .expect("third fragment appends");
    let layout = EditorLayout::new(
        "root.tex",
        LayoutGeneration::new(1),
        vec![
            Piece::new(first_id, 0, 3),
            Piece::new(second_id, 0, 3),
            Piece::new(third_id, 0, 2),
        ],
        &fragments,
    )
    .expect("layout is valid");
    (fragments, layout, [first, second, third])
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
fn source_text_span_preserves_utf8_cursor_and_catcode_seams() {
    let mut stores = Universe::new();
    stores.set_int_param(IntParam::END_LINE_CHAR, -1);
    stores.set_catcode('@', Catcode::Active);
    let mut input = InputStack::new(MemoryInput::new("aé@b"));
    let first = input
        .next_traced_token(&mut stores)
        .expect("valid source")
        .expect("first token");
    assert_eq!(first.token(), Some(char_token('a', Catcode::Letter)));

    let mut text = Vec::new();
    assert_eq!(input.append_source_text_span(&mut stores, &mut text), 1);
    assert_eq!(text[0].token(), Some(char_token('é', Catcode::Other)));
    assert_eq!(
        input.current_source_frame().expect("live frame").column(),
        2
    );

    assert_eq!(input.append_source_text_span(&mut stores, &mut text), 0);
    let active = input
        .next_traced_token(&mut stores)
        .expect("valid source")
        .expect("active token");
    assert_eq!(active.token(), Some(char_token('@', Catcode::Active)));

    stores.set_catcode('b', Catcode::Other);
    assert_eq!(input.append_source_text_span(&mut stores, &mut text), 1);
    assert_eq!(
        text.last().and_then(|word| word.token()),
        Some(char_token('b', Catcode::Other))
    );
    assert_eq!(
        input.current_source_frame().expect("live frame").offset(),
        5
    );
}

#[test]
fn source_text_span_canonicalizes_and_collapses_spaces() {
    let mut stores = Universe::new();
    stores.set_int_param(IntParam::END_LINE_CHAR, -1);
    let mut input = InputStack::new(MemoryInput::new("a   b c"));
    let first = input
        .next_traced_token(&mut stores)
        .expect("valid source")
        .expect("first token");
    assert_eq!(first.token(), Some(char_token('a', Catcode::Letter)));

    let mut text = Vec::new();
    assert_eq!(input.append_source_text_span(&mut stores, &mut text), 4);
    assert_eq!(
        text.iter()
            .filter_map(|word| word.token())
            .collect::<Vec<_>>(),
        vec![
            char_token(' ', Catcode::Space),
            char_token('b', Catcode::Letter),
            char_token(' ', Catcode::Space),
            char_token('c', Catcode::Letter),
        ]
    );
    assert_source_origin(&stores, text[0].origin(), 1, 1, 1);
    assert_source_origin(&stores, text[1].origin(), 4, 1, 4);
    assert_eq!(
        input.current_source_frame().expect("live frame").offset(),
        7
    );
}

#[test]
fn source_text_span_deopts_for_superscript_notation_and_alignment() {
    let mut stores = Universe::new();
    stores.set_int_param(IntParam::END_LINE_CHAR, -1);
    let mut input = InputStack::new(MemoryInput::new("a^^41b"));
    input
        .next_traced_token(&mut stores)
        .expect("valid source")
        .expect("first token");

    let mut text = Vec::new();
    assert_eq!(input.append_source_text_span(&mut stores, &mut text), 0);
    let transformed = input
        .next_traced_token(&mut stores)
        .expect("valid notation")
        .expect("transformed token");
    assert_eq!(transformed.token(), Some(char_token('A', Catcode::Letter)));

    input.begin_alignment();
    assert_eq!(input.append_source_text_span(&mut stores, &mut text), 0);
    input.finish_alignment();
    assert_eq!(input.append_source_text_span(&mut stores, &mut text), 1);
    assert_eq!(text[0].token(), Some(char_token('b', Catcode::Letter)));
}

#[test]
fn source_text_span_summary_resumes_at_the_exact_provenance_seam() {
    let mut stores = Universe::new();
    stores.set_int_param(IntParam::END_LINE_CHAR, -1);
    let mut input = InputStack::new(MemoryInput::new("aébc\\foo"));
    input
        .next_traced_token(&mut stores)
        .expect("valid source")
        .expect("first token");
    let mut text = Vec::new();
    assert_eq!(input.append_source_text_span(&mut stores, &mut text), 3);

    let summary = input.publication_summary(&mut stores);
    let mut restored =
        InputStack::from_summary(&summary, |_, _, _| Ok::<_, ()>(MemoryInput::new("")))
            .expect("source summary restores");
    assert_eq!(restored.summary(), summary);
    let control = restored
        .next_traced_token(&mut stores)
        .expect("valid resumed source")
        .expect("control token");
    assert_eq!(control.token(), Some(cs_token(&mut stores, "foo")));
    assert_source_origin(&stores, control.origin(), 5, 1, 4);
}

#[test]
fn source_text_span_deopts_for_pending_delivery_and_source_transition() {
    let mut stores = Universe::new();
    stores.set_int_param(IntParam::END_LINE_CHAR, -1);
    let mut input = InputStack::new(MemoryInput::new("ab"));
    input
        .next_traced_token(&mut stores)
        .expect("valid outer source")
        .expect("first token");
    let pending = TracedTokenWord::pack(char_token('x', Catcode::Other), OriginId::UNKNOWN);
    assert!(input.push_current_source_pending(pending));
    let mut text = Vec::new();
    assert_eq!(input.append_source_text_span(&mut stores, &mut text), 0);
    assert_eq!(
        input
            .next_traced_token(&mut stores)
            .expect("valid pending delivery"),
        Some(pending)
    );
    assert_eq!(input.append_source_text_span(&mut stores, &mut text), 1);
    assert_eq!(text[0].token(), Some(char_token('b', Catcode::Letter)));

    let mut transition = InputStack::new(MemoryInput::new("ab"));
    transition
        .next_traced_token(&mut stores)
        .expect("valid outer source")
        .expect("outer first token");
    transition.push_source(MemoryInput::new("cd"));
    transition
        .next_traced_token(&mut stores)
        .expect("valid nested source")
        .expect("nested first token");
    text.clear();
    assert_eq!(
        transition.append_source_text_span(&mut stores, &mut text),
        1
    );
    assert_eq!(text[0].token(), Some(char_token('d', Catcode::Letter)));
    assert_eq!(
        transition.append_source_text_span(&mut stores, &mut text),
        0
    );
    assert_eq!(
        transition
            .next_traced_token(&mut stores)
            .expect("nested source pops at its boundary")
            .expect("outer source resumes")
            .token(),
        Some(char_token('b', Catcode::Letter))
    );
}

#[test]
fn pending_paragraph_anchor_requires_direct_root_provenance() {
    let mut fragments = FragmentStore::new();
    let (fragment, _) = fragments
        .append(Arc::from(&b"ab"[..]), 1)
        .expect("fragment appends");
    let layout = EditorLayout::new(
        "root.tex",
        LayoutGeneration::new(1),
        vec![Piece::new(fragment, 0, 2)],
        &fragments,
    )
    .expect("layout is valid");
    let mut input = InputStack::new(MemoryInput::new("ab"));
    input.install_root_layout_cursor(
        LayoutCursor::new(&layout, &fragments).expect("layout cursor freezes"),
    );
    let mut stores = Universe::new();
    stores
        .install_editor_fragments(&fragments, &layout)
        .expect("fragment metadata installs");
    stores.set_int_param(IntParam::END_LINE_CHAR, -1);

    let direct = input
        .next_traced_token(&mut stores)
        .expect("valid source")
        .expect("direct token");
    let expected = stores
        .direct_root_span_for_origin(direct.origin())
        .expect("direct token has rooted source")
        .start_anchor();
    assert!(input.push_current_source_pending(direct));
    assert_eq!(
        input
            .current_root_delivery_anchor(&mut stores)
            .expect("direct anchor"),
        Some(expected)
    );
    assert_eq!(
        input.next_traced_token(&mut stores).expect("pending token"),
        Some(direct)
    );

    let inserted_origin = stores.inserted_origin(
        InsertedOriginKind::NoExpand,
        direct.token().expect("semantic token"),
        direct.origin(),
    );
    let inserted = TracedTokenWord::pack(direct.token().expect("semantic token"), inserted_origin);
    assert!(input.push_current_source_pending(inserted));
    assert_eq!(
        input
            .current_root_delivery_anchor(&mut stores)
            .expect("inserted pending token is not alignable"),
        None
    );
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
fn classic_input_mode_delivers_utf8_bytes_and_resumes_mid_scalar() {
    let mut stores = Universe::new();
    stores.set_int_param(IntParam::END_LINE_CHAR, -1);
    for byte in [0xef, 0xac, 0x80] {
        stores.set_catcode(char::from(byte), Catcode::Other);
    }
    let mut input = InputStack::new(MemoryInput::new("ﬀ"));
    input.set_utf8_input_as_bytes(true);

    assert_eq!(
        input.next_token(&mut stores).expect("first byte"),
        Some(char_token(char::from(0xef), Catcode::Other))
    );
    let summary = input.summary();
    assert!(summary.utf8_input_as_bytes());
    let [InputFrameSummary::Source { source, .. }] = summary.frames() else {
        panic!("expected source frame");
    };
    assert_eq!(source.line_byte_offset(), 1);
    assert!(source.is_resume_complete());

    let mut restored =
        InputStack::from_summary(&summary, |_, _, _| Ok::<_, ()>(MemoryInput::new("")))
            .expect("byte-oriented summary restores");
    assert_eq!(
        restored.next_token(&mut stores).expect("second byte"),
        Some(char_token(char::from(0xac), Catcode::Other))
    );
    assert_eq!(
        restored.next_token(&mut stores).expect("third byte"),
        Some(char_token(char::from(0x80), Catcode::Other))
    );
}

#[test]
fn classic_input_mode_does_not_reencode_a_lossless_byte_projection() {
    let mut stores = Universe::new();
    stores.set_int_param(IntParam::END_LINE_CHAR, -1);
    stores.set_catcode(char::from(0xed), Catcode::Other);
    let projected: String = [b'a', 0xed, b'b'].into_iter().map(char::from).collect();
    let mut input = InputStack::new(MemoryInput::byte_projection(projected));
    input.set_utf8_input_as_bytes(true);

    assert_eq!(
        input.next_token(&mut stores).expect("ASCII token"),
        Some(char_token('a', Catcode::Letter))
    );
    assert_eq!(
        input.next_token(&mut stores).expect("projected byte token"),
        Some(char_token(char::from(0xed), Catcode::Other))
    );
    let summary = input.summary();
    let [InputFrameSummary::Source { source, .. }] = summary.frames() else {
        panic!("expected source frame");
    };
    assert!(source.byte_projection());
    assert_eq!(source.line_byte_offset(), 3);
    assert_eq!(source.column(), 2);
    assert_eq!(
        input.next_token(&mut stores).expect("trailing ASCII token"),
        Some(char_token('b', Catcode::Letter))
    );
}

#[test]
fn classic_input_mode_does_not_reencode_scantokens_characters() {
    let mut stores = Universe::new();
    stores.set_int_param(IntParam::END_LINE_CHAR, -1);
    let mut input = InputStack::new(MemoryInput::scantokens("é"));
    input.set_utf8_input_as_bytes(true);

    assert_eq!(
        input.next_token(&mut stores).expect("scantokens character"),
        Some(char_token('é', Catcode::Other))
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
fn source_depth_tracks_nested_sources_and_summary_restoration() {
    let mut stores = Universe::new();
    stores.set_int_param(IntParam::END_LINE_CHAR, -1);
    let mut input = InputStack::new(MemoryInput::new("a"));
    assert_eq!(input.source_depth(), 1);

    input.push_source(MemoryInput::new(""));
    input.push_source(MemoryInput::new("b"));
    assert_eq!(input.source_depth(), 3);

    assert_eq!(
        input.next_token(&mut stores).expect("nested token"),
        Some(char_token('b', Catcode::Letter))
    );
    assert_eq!(input.source_depth(), 3);
    assert_eq!(
        input.next_token(&mut stores).expect("outer token"),
        Some(char_token('a', Catcode::Letter))
    );
    assert_eq!(input.source_depth(), 1);

    let summary = input.summary();
    let restored = InputStack::from_summary(&summary, |_, _, _| Ok::<_, ()>(MemoryInput::new("")))
        .expect("summary restores");
    assert_eq!(restored.source_depth(), 1);
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
fn classic_input_mode_preserves_invalid_bytes_across_world_input_resume() {
    let mut stores = Universe::new();
    stores.set_int_param(IntParam::END_LINE_CHAR, -1);
    for byte in [0xed, 0x80] {
        stores.set_catcode(char::from(byte), Catcode::Other);
    }
    stores
        .world_mut()
        .set_memory_file("invalid.tex", vec![b'a', 0xed, b'b', b'\n', 0x80])
        .expect("seed invalid input");
    let content = stores
        .world_mut()
        .read_file("invalid.tex")
        .expect("read invalid input bytes");
    let mut input = InputStack::new(super::WorldInput::from_content(content));
    input.set_utf8_input_as_bytes(true);

    assert_eq!(
        input.next_token(&mut stores).expect("ASCII token"),
        Some(char_token('a', Catcode::Letter))
    );
    assert_eq!(
        input.next_token(&mut stores).expect("invalid byte token"),
        Some(char_token(char::from(0xed), Catcode::Other))
    );
    let summary = input.publication_summary(&mut stores);
    let [InputFrameSummary::Source { source, .. }] = summary.frames() else {
        panic!("expected source frame");
    };
    assert!(summary.utf8_input_as_bytes());
    assert!(source.bytes_as_chars());
    assert_eq!(source.line_byte_offset(), 2);
    assert!(source.is_resume_complete());

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
    .expect("restore byte-oriented world input");

    assert_eq!(
        restored
            .next_token(&mut stores)
            .expect("restored ASCII token"),
        Some(char_token('b', Catcode::Letter))
    );
    assert_eq!(
        restored
            .next_token(&mut stores)
            .expect("restored invalid byte on following line"),
        Some(char_token(char::from(0x80), Catcode::Other))
    );
    assert_eq!(
        restored.next_token(&mut stores).expect("end of input"),
        None
    );
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

    assert_eq!(
        input.next_token(&mut stores).expect("outer frame pops"),
        Some(char_token('a', Catcode::Letter))
    );
    let replacement = input.push_token_list(list, TokenListReplayKind::Inserted);
    assert!(!input.contains_token_list_replay_marker(outer));
    assert!(input.contains_token_list_replay_marker(replacement));
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
fn do_endv_stack_walk_accepts_only_empty_frames_above_v_template() {
    let mut stores = Universe::new();
    let list = stores.intern_token_list(&[char_token('x', Catcode::Letter)]);
    let mut input = InputStack::new(MemoryInput::new("a"));
    input.push_token_list(list, TokenListReplayKind::AlignmentVTemplate);

    assert_eq!(
        input.next_token(&mut stores).expect("v-template token"),
        Some(char_token('x', Catcode::Letter))
    );
    input.push_token_list(TokenListId::EMPTY, TokenListReplayKind::Inserted);
    assert!(input.has_exhausted_alignment_v_template(&stores));

    input.push_token_list(list, TokenListReplayKind::Inserted);
    assert!(!input.has_exhausted_alignment_v_template(&stores));
}

#[test]
fn completed_alignment_cell_retires_exhausted_v_template_boundary() {
    let mut stores = Universe::new();
    let template = stores.intern_token_list(&[char_token('x', Catcode::Letter)]);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.begin_alignment();
    input.begin_alignment_cell(None, template, 0);
    let terminator =
        TracedTokenWord::pack(char_token('&', Catcode::AlignmentTab), OriginId::UNKNOWN);
    assert!(input.intercept_alignment_token(
        terminator,
        AlignmentTokenDelivery::Other,
        Some(AlignmentTerminator::Tab),
        0,
    ));
    assert_eq!(
        input.next_token(&mut stores).expect("v-template token"),
        Some(char_token('x', Catcode::Letter))
    );
    input.push_token_list(TokenListId::EMPTY, TokenListReplayKind::Inserted);
    assert!(input.has_exhausted_alignment_v_template(&stores));

    assert_eq!(
        input.finish_terminating_alignment_cell(&stores),
        Some(terminator)
    );
    assert!(!input.has_exhausted_alignment_v_template(&stores));
    input.finish_alignment();
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
fn transient_replay_preserves_inline_origins_and_summarizes_only_live_suffix() {
    let mut stores = Universe::new();
    let first = char_token('x', Catcode::Letter);
    let second = char_token('y', Catcode::Letter);
    let first_origin = stores.source_origin(tex_state::SourceId::new(1), 10, 2, 3);
    let second_origin = stores.source_origin(tex_state::SourceId::new(1), 11, 2, 4);
    let words = vec![
        TracedTokenWord::pack(first, first_origin),
        TracedTokenWord::pack(second, second_origin),
    ];
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_transient_tokens(words, TokenListReplayKind::Inserted);

    let replayed = input
        .next_traced_token(&mut stores)
        .expect("transient replay")
        .expect("first token");
    assert_eq!(replayed.unpack(), Some((first, first_origin)));

    let summary = input.summary();
    let Some(InputFrameSummary::TransientTokenList { tokens, .. }) = summary.frames().last() else {
        panic!("expected transient replay summary");
    };
    assert_eq!(
        tokens.as_ref(),
        &[TracedTokenWord::pack(second, second_origin)]
    );

    let mut restored =
        InputStack::from_summary(&summary, |_, _, _| Ok::<_, ()>(MemoryInput::new("")))
            .expect("restore transient replay");
    let replayed = restored
        .next_traced_token(&mut stores)
        .expect("restored transient replay")
        .expect("remaining token");
    assert_eq!(replayed.unpack(), Some((second, second_origin)));
}

#[test]
fn transient_replay_buffers_return_to_pool_on_exhaustion_and_abort() {
    let mut stores = Universe::new();
    let token = char_token('x', Catcode::Letter);
    let word = TracedTokenWord::pack(token, OriginId::UNKNOWN);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_transient_tokens(vec![word], TokenListReplayKind::Inserted);

    assert_eq!(
        input.next_token(&mut stores).expect("transient replay"),
        Some(token)
    );
    assert_eq!(input.next_token(&mut stores).expect("retire replay"), None);
    assert_eq!(input.transient_buffer_pool.len(), 1);
    assert!(input.take_transient_token_buffer().is_empty());

    let outer = input.push_transient_tokens(vec![word], TokenListReplayKind::Inserted);
    input.push_transient_tokens(vec![word], TokenListReplayKind::NoExpand);
    assert!(input.abort_token_list_replay(outer));
    assert_eq!(input.transient_buffer_pool.len(), 2);
}

#[test]
fn replay_abort_removes_nested_source_and_condition_frames() {
    let mut input = InputStack::new(MemoryInput::new("outer"));
    let marker = input.push_transient_tokens(Vec::new(), TokenListReplayKind::OutputRoutine);
    input.push_source(MemoryInput::new("nested"));
    input.push_condition(ConditionFrameSummary::new_if(condition_context(), true));

    assert_eq!(input.source_depth(), 2);
    assert_eq!(input.condition_depth(), 1);
    assert!(input.abort_token_list_replay(marker));
    assert_eq!(input.source_depth(), 1);
    assert_eq!(input.condition_depth(), 0);
    assert!(!input.contains_token_list_replay_marker(marker));
}

#[test]
fn transient_replay_pool_drops_exceptionally_large_buffers() {
    let mut stores = Universe::new();
    let mut words = Vec::with_capacity(super::TRANSIENT_BUFFER_POOL_MAX_CAPACITY + 1);
    words.push(TracedTokenWord::pack(
        char_token('x', Catcode::Letter),
        OriginId::UNKNOWN,
    ));
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_transient_tokens(words, TokenListReplayKind::Inserted);

    assert!(
        input
            .next_token(&mut stores)
            .expect("transient replay")
            .is_some()
    );
    assert_eq!(input.next_token(&mut stores).expect("retire replay"), None);
    assert!(input.transient_buffer_pool.is_empty());
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
fn horizontal_macro_text_span_includes_canonical_space_tokens() {
    let mut stores = Universe::new();
    let tokens = [
        char_token('a', Catcode::Letter),
        char_token(' ', Catcode::Space),
        char_token('b', Catcode::Other),
    ];
    let token_list = stores.intern_token_list(&tokens);
    let origin = stores.source_origin(tex_state::SourceId::new(1), 10, 2, 3);
    let origins = stores.allocate_origin_list(&[origin; 3]);
    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_macro_body_with_origins(token_list, origins, MacroArguments::new());
    let mut text = Vec::new();

    assert_eq!(input.append_macro_text_span(&stores, &mut text), 3);
    assert_eq!(
        text.iter()
            .filter_map(|word| word.token())
            .collect::<Vec<_>>(),
        tokens
    );
    assert!(text.iter().all(|word| word.origin() == origin));
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
    let body_origin = stores.source_origin(tex_state::SourceId::new(1), 10, 1, 1);
    let argument_origin = stores.source_origin(tex_state::SourceId::new(2), 20, 2, 1);
    let body_origins = stores.allocate_origin_list(&[body_origin; 5]);
    let argument_words = argument_tokens
        .into_iter()
        .map(|token| TracedTokenWord::pack(token, argument_origin))
        .collect();
    let mut ranges = [None; MACRO_ARGUMENT_SLOTS];
    ranges[0] = Some(MacroArgumentRange::new(0, 2));
    let arguments = MacroArguments::from_parts(argument_words, ranges);

    let mut input = InputStack::new(MemoryInput::new(""));
    input.push_macro_body_with_origins(body, body_origins, arguments);
    let summary = input.summary();
    let mut input = InputStack::from_summary(&summary, |_, _, _| Ok::<_, ()>(MemoryInput::new("")))
        .expect("restore macro arguments by value");
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
        0
    );
    let first_argument = input
        .next_traced_token(&mut stores)
        .expect("argument replay")
        .expect("first argument token");
    let second_argument = input
        .next_traced_token(&mut stores)
        .expect("argument replay")
        .expect("second argument token");
    assert_eq!(
        first_argument.unpack(),
        Some((argument_tokens[0], argument_origin))
    );
    assert_eq!(
        second_argument.unpack(),
        Some((argument_tokens[1], argument_origin))
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
        [body_tokens[0], body_tokens[1], body_tokens[3]]
    );
    assert_eq!(
        stores.origin_list(origin_list),
        [body_origin, body_origin, body_origin]
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

#[cfg(feature = "profiling-stats")]
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
    assert_eq!(stats.builder_append_timer_samples, 1);
}

#[cfg(feature = "profiling-stats")]
#[test]
fn expansion_timers_sample_one_event_per_1024() {
    let mut events = 0;
    let sampled = (0..2050)
        .filter(|_| super::should_sample_timer(&mut events))
        .collect::<Vec<_>>();

    assert_eq!(sampled, vec![0, 1024, 2048]);
    assert_eq!(events, 2050);
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

fn collect_tokens(lexer: &mut Lexer, stores: &mut impl ExpansionState) -> Vec<Token> {
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
