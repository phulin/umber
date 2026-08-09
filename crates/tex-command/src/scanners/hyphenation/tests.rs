use tex_state::meaning::Meaning;
use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};

use super::{HyphenationDataKind, ScannedHyphenationData};
use crate::input::{
    ReplayTrace, RetirementBehavior, SharedTokenBuffer, TokenBehavior, TokenPayload,
};
use crate::{
    CommandHostCapabilities, CommandHostContext, CommandObservation, CommandObserver,
    CommandProcessor, CommandState,
};

#[derive(Default)]
struct Recorder(Vec<CommandObservation>);

impl CommandObserver for Recorder {
    fn committed(&mut self, observation: CommandObservation) {
        self.0.push(observation);
    }
}

fn char_token(ch: char) -> Token {
    Token::Char {
        ch,
        cat: match ch {
            '{' => Catcode::BeginGroup,
            '}' => Catcode::EndGroup,
            ' ' => Catcode::Space,
            'a'..='z' | 'A'..='Z' => Catcode::Letter,
            _ => Catcode::Other,
        },
    }
}

fn push(command: &mut CommandState, text: &str) {
    command.push_token_level(
        TokenPayload::Transient(SharedTokenBuffer::new(
            text.chars()
                .map(|ch| TracedTokenWord::pack(char_token(ch), OriginId::UNKNOWN))
                .collect::<Vec<_>>(),
        )),
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::BackedUp,
    );
}

fn words(scanned: &ScannedHyphenationData) -> Vec<String> {
    scanned
        .words
        .iter()
        .map(|word| word.iter().collect::<String>())
        .collect()
}

#[test]
fn initex_exception_scan_defers_saved_codes_until_the_trie_is_ready() {
    fn scan(patterns_open: bool) -> Vec<String> {
        let mut command = CommandState::default();
        let mut universe = crate::test_harness::universe();
        universe.set_lccode('B', u32::from(b'b'));
        universe.save_hyphenation_codes(0, [('q', 'q'), ('p', 'p'), ('B', 'r')]);
        if !patterns_open {
            universe.close_hyphenation_patterns();
        }
        let mut capabilities = CommandHostCapabilities::default();
        push(&mut command, "{qqB-pp}");
        let mut processor = CommandProcessor::new(
            &mut command,
            universe.command_context(),
            CommandHostContext::new(&mut capabilities),
        );

        words(
            &processor
                .scan_hyphenation_data(HyphenationDataKind::Exceptions)
                .expect("exception group scans"),
        )
    }

    // pdfTeX §934: INITEX leaves `hyph_index=0` while `trie_not_ready`, but
    // a loaded/initialized trie selects the language's saved hyphen codes.
    assert_eq!(scan(true), ["qqb-pp"]);
    assert_eq!(scan(false), ["qqr-pp"]);
}

#[test]
fn patterns_keep_exactly_pdftexs_first_63_characters() {
    for supplied in [63, 64] {
        let mut command = CommandState::default();
        let mut universe = crate::test_harness::universe();
        let mut capabilities = CommandHostCapabilities::default();
        push(&mut command, &format!("{{{}1}}", "a".repeat(supplied)));
        let mut processor = CommandProcessor::new(
            &mut command,
            universe.command_context(),
            CommandHostContext::new(&mut capabilities),
        );

        let scanned = processor
            .scan_hyphenation_data(HyphenationDataKind::Patterns)
            .expect("pattern group scans");
        let pattern = scanned.patterns.first().expect("one pattern");
        assert_eq!(pattern.letters, vec!['a'; 63], "supplied={supplied}");
        assert_eq!(pattern.values.len(), 64, "supplied={supplied}");
        assert_eq!(
            pattern.values[63], 0,
            "once k reaches 63, subsequent digits are discarded too"
        );
    }
}

/// TeX82 §473's `scan_toks` is the only routine that sets
/// `scanner_status:=absorbing`, and neither §934's `new_hyph_exceptions` nor
/// §960's `new_patterns` calls it: both run a plain `get_x_token` loop after
/// §403's `scan_left_brace`. An absorbing episode here would publish a
/// scanner-status transition ahead of the compulsory brace's own delivery.
#[test]
fn hyphenation_data_scan_never_enters_absorbing() {
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe();
    let mut capabilities = CommandHostCapabilities::default();
    let mut recorder = Recorder::default();
    push(&mut command, "{ab cd}");
    let mut processor = CommandProcessor::new(
        &mut command,
        universe.command_context(),
        CommandHostContext::new(&mut capabilities),
    )
    .with_observer(&mut recorder);

    let scanned = processor
        .scan_hyphenation_data(HyphenationDataKind::Patterns)
        .expect("pattern group scans");

    assert_eq!(words(&scanned), vec!["ab".to_owned(), "cd".to_owned()]);
    assert!(
        !recorder.0.iter().any(|observation| matches!(
            observation,
            CommandObservation::ScannerStatus(status)
                if status.from == "absorbing" || status.to == "absorbing"
        )),
        "hyphenation data must not publish an absorbing episode"
    );
}

#[test]
fn hyphenation_data_continues_after_section_403_inserted_left_brace() {
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe();
    let mut capabilities = CommandHostCapabilities::default();
    push(&mut command, "ab}");
    let mut processor = CommandProcessor::new(
        &mut command,
        universe.command_context(),
        CommandHostContext::new(&mut capabilities),
    );

    let scanned = processor
        .scan_hyphenation_data(HyphenationDataKind::Exceptions)
        .expect("inserted opener starts the exception list");

    assert_eq!(words(&scanned), vec!["ab".to_owned()]);
    assert_eq!(
        processor.command.alignment.align_state,
        crate::processor::TOP_LEVEL_ALIGN_STATE
    );
}

/// §961's `othercases` diagnoses a `left_brace` as "Bad \\patterns" and
/// resumes the same loop, so the group has no nested levels: the next
/// `right_brace` ends the scan even though a `{` was seen. Collecting this
/// group through `scan_toks` instead tracked a brace depth TeX never
/// maintains and swallowed that closing brace.
#[test]
fn nested_left_brace_opens_no_level_and_the_next_right_brace_ends_the_scan() {
    let mut command = CommandState::default();
    let mut universe = crate::test_harness::universe();
    let mut capabilities = CommandHostCapabilities::default();
    push(&mut command, "{a{b}c}");
    let mut processor = CommandProcessor::new(
        &mut command,
        universe.command_context(),
        CommandHostContext::new(&mut capabilities),
    );

    let scanned = processor
        .scan_hyphenation_data(HyphenationDataKind::Patterns)
        .expect("pattern group scans");

    assert_eq!(words(&scanned), vec!["ab".to_owned()]);
    assert_eq!(
        processor
            .get_token()
            .expect("trailing token delivers")
            .expect("trailing token exists")
            .spelling()
            .semantic_token(),
        char_token('c'),
        "the scan stopped at the first right brace, leaving the rest unread"
    );
}

/// §935 accepts `char_given` as a word character; §961's case list does not,
/// so the same token is `othercases` inside `\patterns`. The two scans are
/// separate values precisely so this difference is representable.
#[test]
fn char_given_is_a_hyphenation_word_character_but_not_a_pattern_one() {
    for (kind, expected) in [
        (HyphenationDataKind::Exceptions, vec!["axb".to_owned()]),
        (HyphenationDataKind::Patterns, vec!["ab".to_owned()]),
    ] {
        let mut command = CommandState::default();
        let mut universe = crate::test_harness::universe();
        let given = universe.intern("givenx").symbol();
        universe.set_meaning(given, Meaning::CharGiven('x'));
        command.push_token_level(
            TokenPayload::Transient(SharedTokenBuffer::new(vec![
                TracedTokenWord::pack(char_token('{'), OriginId::UNKNOWN),
                TracedTokenWord::pack(char_token('a'), OriginId::UNKNOWN),
                TracedTokenWord::pack(Token::Cs(given), OriginId::UNKNOWN),
                TracedTokenWord::pack(char_token('b'), OriginId::UNKNOWN),
                TracedTokenWord::pack(char_token('}'), OriginId::UNKNOWN),
            ])),
            TokenBehavior::Ordinary,
            RetirementBehavior::Pop,
            ReplayTrace::BackedUp,
        );
        let mut capabilities = CommandHostCapabilities::default();
        let mut processor = CommandProcessor::new(
            &mut command,
            universe.command_context(),
            CommandHostContext::new(&mut capabilities),
        );

        let scanned = processor.scan_hyphenation_data(kind).expect("group scans");

        assert_eq!(words(&scanned), expected, "{kind:?}");
    }
}

#[test]
fn othercases_print_errors_without_inventing_events_and_preserve_the_partial_word() {
    for (kind, expected_message) in [
        (
            HyphenationDataKind::Exceptions,
            "Improper \\hyphenation will be flushed",
        ),
        (HyphenationDataKind::Patterns, "Bad \\patterns"),
    ] {
        let mut command = CommandState::default();
        let mut universe = crate::test_harness::universe();
        let bad = universe.intern("bad").symbol();
        universe.set_meaning(bad, Meaning::Relax);
        command.push_token_level(
            TokenPayload::Transient(SharedTokenBuffer::new(vec![
                TracedTokenWord::pack(char_token('{'), OriginId::UNKNOWN),
                TracedTokenWord::pack(char_token('a'), OriginId::UNKNOWN),
                TracedTokenWord::pack(Token::Cs(bad), OriginId::UNKNOWN),
                TracedTokenWord::pack(char_token('b'), OriginId::UNKNOWN),
                TracedTokenWord::pack(char_token('}'), OriginId::UNKNOWN),
            ])),
            TokenBehavior::Ordinary,
            RetirementBehavior::Pop,
            ReplayTrace::BackedUp,
        );
        let mut capabilities = CommandHostCapabilities::default();
        let mut recorder = Recorder::default();
        {
            let mut processor = CommandProcessor::new(
                &mut command,
                universe.command_context(),
                CommandHostContext::new(&mut capabilities),
            )
            .with_observer(&mut recorder);
            let scanned = processor.scan_hyphenation_data(kind).expect("group scans");
            assert_eq!(words(&scanned), vec!["ab"]);
        }
        assert!(
            !recorder
                .0
                .iter()
                .any(|event| matches!(event, CommandObservation::Diagnostic(_))),
            "§§936/961 have no schema-v1 diagnostic observation"
        );
        let committed = universe
            .world()
            .memory_terminal_output()
            .map(String::from_utf8_lossy)
            .unwrap_or_default();
        let pending: String = universe
            .world()
            .effect_records()
            .iter()
            .filter_map(|effect| match effect {
                tex_state::EffectRecord::StreamWrite {
                    sink:
                        tex_state::PrintSink::Terminal
                        | tex_state::PrintSink::TerminalAndLog
                        | tex_state::PrintSink::Log,
                    text,
                } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        let terminal = committed.into_owned() + &pending;
        assert!(
            terminal.contains(expected_message),
            "the §82 error remains visible on the terminal: {:?}",
            terminal
        );
    }
}
