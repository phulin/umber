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
    arena: &mut ArgumentArena,
    writer: &mut ArgumentCursor,
    words: impl IntoIterator<Item = TracedTokenWord>,
) {
    for word in words {
        arena
            .append_argument_token(writer, ClassifiedToken::from_word(word, None), true)
            .expect("argument word");
    }
}

#[test]
fn admitted_brace_delta_drives_nested_depth_and_first_scan_facts_once() {
    let mut arena = ArgumentArena::default();
    let mut matching = arena.begin_match().expect("macro match");
    let mut writer = arena.begin_argument(&matching);
    let words = [
        brace('{', Catcode::BeginGroup),
        brace('[', Catcode::BeginGroup),
        word('p'),
        brace(']', Catcode::EndGroup),
        brace('}', Catcode::EndGroup),
    ];
    let paragraph_token = word('p').semantic_token();
    for (word, expected_depth) in words.into_iter().zip([1, 2, 2, 1, 0]) {
        let depth = arena
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
    arena
        .publish_argument(&mut matching, writer)
        .expect("argument range");

    let address = arena.region(matching.region).as_ptr();
    let mut parameters = Vec::new();
    let region = arena
        .publish(matching, &mut parameters)
        .expect("publish coordinates");
    assert_eq!(parameters, [ArgumentSpan { start: 1, len: 3 }]);
    assert_eq!(&arena.region(region)[1..4], &words[1..4]);
    assert_eq!(
        arena.region(region).as_ptr(),
        address,
        "publication never copies the payload"
    );
    arena.release(region);
    assert!(arena.is_empty());
}

#[test]
fn publication_keeps_delimiter_recovery_and_empty_argument_ranges() {
    let mut arena = ArgumentArena::default();
    let mut matching = arena.begin_match().expect("macro match");
    let mut first = arena.begin_argument(&matching);
    append_all(&mut arena, &mut first, [word('a'), word('b')]);
    arena
        .append_delimiter_word(&mut first, word('.'))
        .expect("delimiter candidate");
    assert_eq!(arena.delimiter_prefix_len(&first), Ok(1));
    assert_eq!(arena.delimiter_prefix_word(&first, 0), Ok(word('.')));
    arena
        .truncate_delimiter_suffix(&mut first)
        .expect("completed delimiter");
    arena
        .publish_argument(&mut matching, first)
        .expect("first argument");

    let mut second = arena.begin_argument(&matching);
    append_all(&mut arena, &mut second, [word('c')]);
    arena
        .append_delimiter_word(&mut second, word('x'))
        .expect("delimiter candidate");
    arena
        .reveal_delimiter_words(&mut second, 1)
        .expect("mismatched delimiter prefix");
    arena
        .publish_argument(&mut matching, second)
        .expect("second argument");

    let empty = arena.begin_argument(&matching);
    arena
        .publish_argument(&mut matching, empty)
        .expect("empty argument");

    let mut parameters = vec![ArgumentSpan { start: 7, len: 7 }];
    let region = arena
        .publish(matching, &mut parameters)
        .expect("publish ranges");
    assert_eq!(
        arena.region(region),
        [word('a'), word('b'), word('c'), word('x')]
    );
    assert_eq!(
        parameters[1..],
        [
            ArgumentSpan { start: 0, len: 2 },
            ArgumentSpan { start: 2, len: 2 },
            ArgumentSpan { start: 4, len: 0 }
        ]
    );
    arena.release(region);
    assert!(arena.is_empty());
}

#[test]
fn trimmed_match_iterator_stops_at_both_stored_range_boundaries() {
    let mut arena = ArgumentArena::default();
    let matching = arena.begin_match().expect("macro match");
    let mut writer = arena.begin_argument(&matching);
    append_all(
        &mut arena,
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
    let mut words = arena.match_words(&writer).expect("trimmed words");
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
        let mut arena = ArgumentArena::default();
        let matching = arena.begin_match().expect("macro match");
        let mut writer = arena.begin_argument(&matching);
        append_all(&mut arena, &mut writer, words);
        assert_eq!(writer.facts().removable_outer_group(), expected);
    }
}

#[test]
fn forwarded_capture_survives_parent_release_and_abort_preserves_live_parent() {
    let mut arena = ArgumentArena::default();
    let mut parent = arena.begin_match().expect("parent match");
    let mut cursor = arena.begin_argument(&parent);
    // More than the destination's initial capacity proves forwarding stays
    // valid when its final vector reallocates.
    let words = vec![word('p'); 8_192];
    append_all(&mut arena, &mut cursor, words.iter().copied());
    arena
        .publish_argument(&mut parent, cursor)
        .expect("parent span");
    let parent = arena
        .publish(parent, &mut Vec::new())
        .expect("parent owner");
    let mut child = arena.begin_match().expect("child match");
    let mut cursor = arena.begin_argument(&child);
    arena
        .append_region(&mut cursor, parent, 0..words.len())
        .expect("forward final words");
    arena.release(parent);
    assert_eq!(
        arena
            .match_words(&cursor)
            .expect("child after parent retirement")
            .collect::<Vec<_>>(),
        words
    );
    arena
        .publish_argument(&mut child, cursor)
        .expect("child span");
    let child = arena.publish(child, &mut Vec::new()).expect("child owner");
    let failed = arena.begin_match().expect("next nested match");
    let mut cursor = arena.begin_argument(&failed);
    append_all(&mut arena, &mut cursor, [word('x')]);
    arena.abort_capture();
    assert_eq!(
        arena.region(child),
        words,
        "failed nested matching preserves its live parent's words"
    );
    arena.release(child);
    assert!(arena.is_empty());
}

#[test]
fn checkpoint_reset_excludes_pool_semantics_and_retains_capacity_once() {
    let mut stack = crate::input::InputStack::<()>::default();
    let matching = stack.arguments.begin_match().expect("capture");
    let mut cursor = stack.arguments.begin_argument(&matching);
    append_all(&mut stack.arguments, &mut cursor, vec![word('x'); 1_024]);
    let retained = stack.arguments.retained_bytes();
    let capture = crate::input::InputStack::default().checkpoint_copy();
    stack.install_checkpoint(&capture);
    assert!(stack.arguments.is_empty());
    assert_eq!(
        stack.arguments.retained_bytes(),
        retained,
        "restore reuses the engine pool"
    );
    assert_eq!(
        stack, capture,
        "vacant allocator slots are not semantic state"
    );
    use core::hash::Hasher;
    let hash = |stack: &crate::input::InputStack<()>| {
        let mut state = std::collections::hash_map::DefaultHasher::new();
        stack.hash(&mut state);
        state.finish()
    };
    assert_eq!(hash(&stack), hash(&capture));
    let checkpoint = stack.checkpoint_copy();
    assert_eq!(
        checkpoint.arguments.retained_bytes(),
        0,
        "named captures never clone the warmed pool"
    );
    let next = stack.arguments.begin_match().expect("reused capture");
    let mut cursor = stack.arguments.begin_argument(&next);
    append_all(&mut stack.arguments, &mut cursor, vec![word('y'); 1_024]);
    assert_eq!(stack.arguments.retained_bytes(), retained);
    stack.arguments.abort_capture();
}
