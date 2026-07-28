use tex_state::Universe;
use tex_state::meaning::Meaning;
use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};

use super::{HyphenationDataKind, ScannedHyphenationData};
use crate::input::{
    ReplayTrace, RetirementBehavior, SharedTokenBuffer, TokenBehavior, TokenPayload,
};
use crate::{
    CommandHostCapabilities, CommandHostContext, CommandObservation, CommandObserver,
    CommandProcessor, CommandRuntime, CommandState,
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

/// TeX82 §473's `scan_toks` is the only routine that sets
/// `scanner_status:=absorbing`, and neither §934's `new_hyph_exceptions` nor
/// §960's `new_patterns` calls it: both run a plain `get_x_token` loop after
/// §403's `scan_left_brace`. An absorbing episode here would publish a
/// scanner-status transition ahead of the compulsory brace's own delivery.
#[test]
fn hyphenation_data_scan_never_enters_absorbing() {
    let mut command = CommandState::default();
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new();
    let mut capabilities = CommandHostCapabilities::default();
    let mut recorder = Recorder::default();
    push(&mut command, "{ab cd}");
    let mut processor = CommandProcessor::new(
        &mut command,
        &mut runtime,
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
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new();
    let mut capabilities = CommandHostCapabilities::default();
    push(&mut command, "ab}");
    let mut processor = CommandProcessor::new(
        &mut command,
        &mut runtime,
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
    let mut runtime = CommandRuntime::default();
    let mut universe = Universe::new();
    let mut capabilities = CommandHostCapabilities::default();
    push(&mut command, "{a{b}c}");
    let mut processor = CommandProcessor::new(
        &mut command,
        &mut runtime,
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
        let mut runtime = CommandRuntime::default();
        let mut universe = Universe::new();
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
            &mut runtime,
            universe.command_context(),
            CommandHostContext::new(&mut capabilities),
        );

        let scanned = processor.scan_hyphenation_data(kind).expect("group scans");

        assert_eq!(words(&scanned), expected, "{kind:?}");
    }
}

#[test]
fn othercases_report_typed_diagnostics_and_preserve_the_partial_word() {
    for (kind, expected_diagnostic) in [
        (HyphenationDataKind::Exceptions, "improper_hyphenation"),
        (HyphenationDataKind::Patterns, "bad_patterns"),
    ] {
        let mut command = CommandState::default();
        let mut runtime = CommandRuntime::default();
        let mut universe = Universe::new();
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
        let mut processor = CommandProcessor::new(
            &mut command,
            &mut runtime,
            universe.command_context(),
            CommandHostContext::new(&mut capabilities),
        )
        .with_observer(&mut recorder);

        let scanned = processor.scan_hyphenation_data(kind).expect("group scans");
        assert_eq!(words(&scanned), vec!["ab"]);
        assert!(recorder.0.iter().any(|event| matches!(
            event,
            CommandObservation::Diagnostic(record)
                if record.diagnostic == expected_diagnostic
        )));
    }
}
