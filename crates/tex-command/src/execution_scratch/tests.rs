use super::*;
use crate::input::ArgumentSpan;
use tex_state::token::{Catcode, OriginId, Token, TokenWord};

fn word(ch: char) -> TracedTokenWord {
    TracedTokenWord::pack(
        Token::Char {
            ch,
            cat: Catcode::Other,
        },
        OriginId::UNKNOWN,
    )
}

fn brace(ch: char, cat: Catcode) -> TracedTokenWord {
    TracedTokenWord::pack(Token::Char { ch, cat }, OriginId::UNKNOWN)
}

fn append_all(
    scratch: &mut ExecutionScratch<()>,
    writer: &mut MacroArgumentWriter<()>,
    words: impl IntoIterator<Item = TracedTokenWord>,
) {
    for word in words {
        scratch
            .append_argument_token(writer, ClassifiedToken::from_word(word, None), true)
            .expect("argument word");
    }
}

#[test]
fn admitted_brace_delta_drives_nested_depth_and_first_scan_facts_once() {
    let mut scratch = ExecutionScratch::<()>::default();
    let matching = scratch.begin_macro_match().expect("macro match");
    let mut writer = scratch
        .begin_argument_writer(&matching)
        .expect("argument writer");
    let words = [
        brace('{', Catcode::BeginGroup),
        brace('[', Catcode::BeginGroup),
        word('p'),
        brace(']', Catcode::EndGroup),
        brace('}', Catcode::EndGroup),
    ];
    let paragraph_token = word('p').semantic_token();
    for (word, expected_depth) in words.into_iter().zip([1, 2, 2, 1, 0]) {
        let depth = scratch
            .append_argument_token(
                &mut writer,
                ClassifiedToken::from_word(word, Some(TokenWord::pack(paragraph_token))),
                true,
            )
            .expect("argument word");
        assert_eq!(depth, expected_depth);
        assert_eq!(writer.brace_depth(), expected_depth);
    }
    let facts = writer.facts();
    assert!(facts.rejects_non_long_paragraph());
    assert!(facts.removable_outer_group());

    writer
        .strip_outer_group()
        .expect("outer group strips by metadata");
    assert_eq!(writer.visible_len(), Ok(3));
    scratch.publish_argument(writer).expect("argument range");

    let mut stack = vec![word('s')];
    let mut parameters = Vec::new();
    let count = scratch
        .transfer_macro_match(matching, &mut stack, &mut parameters)
        .expect("argument transfer");
    assert_eq!(count, 1);
    assert_eq!(parameters, [ArgumentSpan { start: 1, len: 3 }]);
    assert_eq!(stack[1..], words[1..4]);
    assert!(scratch.is_quiescent());
}

#[test]
fn transfer_copies_every_argument_after_a_truncated_delimiter() {
    let mut scratch = ExecutionScratch::<()>::default();
    let matching = scratch.begin_macro_match().expect("macro match");
    let mut first = scratch
        .begin_argument_writer(&matching)
        .expect("first writer");
    append_all(&mut scratch, &mut first, [word('a'), word('b')]);
    scratch
        .append_delimiter_word(&mut first, word('.'))
        .expect("delimiter candidate");
    assert_eq!(scratch.delimiter_prefix_len(&first), Ok(1));
    assert_eq!(scratch.delimiter_prefix_word(&first, 0), Ok(word('.')));
    scratch
        .truncate_delimiter_suffix(&mut first)
        .expect("completed delimiter");
    scratch.publish_argument(first).expect("first argument");

    let mut second = scratch
        .begin_argument_writer(&matching)
        .expect("second writer");
    append_all(&mut scratch, &mut second, [word('c')]);
    scratch
        .append_delimiter_word(&mut second, word('x'))
        .expect("delimiter candidate");
    scratch
        .reveal_delimiter_words(&mut second, 1)
        .expect("mismatched delimiter prefix");
    scratch.publish_argument(second).expect("second argument");

    let empty = scratch
        .begin_argument_writer(&matching)
        .expect("empty writer");
    scratch.publish_argument(empty).expect("empty argument");

    let mut stack = Vec::new();
    let mut parameters = vec![ArgumentSpan { start: 7, len: 7 }];
    scratch
        .transfer_macro_match(matching, &mut stack, &mut parameters)
        .expect("argument transfer");
    assert_eq!(stack, [word('a'), word('b'), word('c'), word('x')]);
    assert_eq!(
        parameters[1..],
        [
            ArgumentSpan { start: 0, len: 2 },
            ArgumentSpan { start: 2, len: 2 },
            ArgumentSpan { start: 4, len: 0 },
        ]
    );
    assert!(scratch.is_quiescent());
}

#[test]
fn matching_never_nests_and_discard_closes_the_match() {
    let mut scratch = ExecutionScratch::<()>::default();
    let matching = scratch.begin_macro_match().expect("macro match");
    assert_eq!(
        scratch.begin_macro_match().map(drop),
        Err(ScratchError::InvalidCoordinate)
    );
    let mut writer = scratch
        .begin_argument_writer(&matching)
        .expect("argument writer");
    append_all(&mut scratch, &mut writer, [word('a')]);
    assert_eq!(
        scratch.begin_argument_writer(&matching).map(drop),
        Err(ScratchError::InvalidCoordinate),
        "one argument is written at a time"
    );
    scratch.discard_macro_match(matching).expect("discard");
    assert!(scratch.is_quiescent());
    let matching = scratch.begin_macro_match().expect("next match");
    scratch.discard_macro_match(matching).expect("discard");
}

#[test]
fn trimmed_match_iterator_stops_at_both_stored_range_boundaries() {
    let mut scratch = ExecutionScratch::<()>::default();
    let matching = scratch.begin_macro_match().expect("macro match");
    let mut writer = scratch
        .begin_argument_writer(&matching)
        .expect("argument writer");
    append_all(
        &mut scratch,
        &mut writer,
        [
            brace('{', Catcode::BeginGroup),
            word('x'),
            brace('}', Catcode::EndGroup),
            word('z'),
        ],
    );
    // The last word models unrelated material beyond the argument. Trimming
    // excludes the outer pair, so neither boundary may leak through the
    // exact-size observation iterator.
    writer.set_end_trim(1);
    writer.strip_outer_group().expect("outer group trim");
    let mut words = scratch.match_words(&writer).expect("trimmed words");
    assert_eq!(words.len(), 1);
    assert_eq!(words.next(), Some(word('x')));
    assert_eq!(words.len(), 0);
    assert_eq!(words.next(), None);
}

#[test]
fn outer_group_fact_rejects_empty_one_token_and_trailing_material() {
    let cases = [
        (Vec::new(), false),
        (vec![word('x')], false),
        (
            vec![
                brace('{', Catcode::BeginGroup),
                word('x'),
                brace('}', Catcode::EndGroup),
            ],
            true,
        ),
        (
            vec![
                brace('{', Catcode::BeginGroup),
                word('x'),
                brace('}', Catcode::EndGroup),
                word('y'),
            ],
            false,
        ),
    ];
    for (words, expected) in cases {
        let mut scratch = ExecutionScratch::<()>::default();
        let matching = scratch.begin_macro_match().expect("macro match");
        let mut writer = scratch
            .begin_argument_writer(&matching)
            .expect("argument writer");
        append_all(&mut scratch, &mut writer, words);
        assert_eq!(writer.facts().removable_outer_group(), expected);
    }
}

#[test]
fn undelimited_admission_completes_on_its_sole_token_or_closing_brace() {
    crate::test_harness::with_universe(|universe| {
        let context = universe.command_context().expect("command context");
        let char_word = |ch, cat| TokenWord::pack(Token::Char { ch, cat });
        let letter = |ch| char_word(ch, Catcode::Letter);
        let open = char_word('{', Catcode::BeginGroup);
        let close = char_word('}', Catcode::EndGroup);
        let admission = || {
            ArgumentRunAdmission::with_commands(&context, None, None, 0, 1).completing_undelimited()
        };

        // TeX82 §392: a sole ordinary token is the whole argument.
        let mut run = admission();
        assert!(run.admits(letter('x')));
        assert!(!run.admits(letter('y')), "the argument is already complete");

        // §394: a group completes at its own closing brace, and interior
        // groups balance within it.
        let mut run = admission();
        for word in [open, open, letter('a'), close, letter('b'), close] {
            assert!(run.admits(word));
        }
        assert!(!run.admits(letter('c')), "the argument is already complete");
        assert_eq!(run.brace_delta(), 0);

        // §393's skipped leading space and §395's stray closing brace stay
        // with the scalar matcher.
        assert!(!admission().admits(char_word(' ', Catcode::Space)));
        assert!(!admission().admits(close));

        // An interior run resumes inside the group and still completes it.
        let mut run = ArgumentRunAdmission::with_commands(&context, None, None, 1, 1)
            .completing_undelimited();
        assert!(run.admits(char_word(' ', Catcode::Space)));
        assert!(run.admits(close));
        assert!(!run.admits(letter('d')));
    });
}
