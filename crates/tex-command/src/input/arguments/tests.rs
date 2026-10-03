use super::*;
use tex_state::token::{Catcode, OriginId, Token};

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
    arena: &mut ArgumentArena<()>,
    writer: &mut ArgumentCursor,
    words: impl IntoIterator<Item = TracedTokenWord>,
) {
    for word in words {
        arena
            .append_argument_word(writer, word)
            .expect("argument word");
    }
}

#[test]
fn admitted_brace_delta_drives_nested_depth_and_first_scan_facts_once() {
    let mut arena = ArgumentArena::<()>::default();
    let mut matching = arena.begin_match().expect("macro match");
    let mut writer = arena.begin_argument(&matching);
    let words = [
        brace('{', Catcode::BeginGroup),
        brace('[', Catcode::BeginGroup),
        word('p'),
        brace(']', Catcode::EndGroup),
        brace('}', Catcode::EndGroup),
    ];
    for (word, expected_depth) in words.into_iter().zip([1, 2, 2, 1, 0]) {
        let depth = arena
            .append_argument_word(&mut writer, word)
            .expect("argument word");
        assert_eq!(depth, expected_depth);
        assert_eq!(writer.brace_depth(), expected_depth);
    }
    assert!(writer.removable_outer_group());

    writer
        .strip_outer_group()
        .expect("outer group strips by metadata");
    assert_eq!(writer.visible_len(), Ok(3));
    arena
        .publish_argument(&mut matching, writer)
        .expect("argument range");

    let region = arena.publish(matching);
    assert_eq!(
        arena.words(region, 0).iter().collect::<Vec<_>>(),
        words[1..4]
    );
    assert_eq!(arena.values(region)[0].len(), 3);
    arena.release(region);
    assert!(arena.is_empty());
}

#[test]
fn publication_keeps_delimiter_recovery_and_empty_argument_ranges() {
    let mut arena = ArgumentArena::<()>::default();
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

    let region = arena.publish(matching);
    assert_eq!(
        arena.words(region, 0).iter().collect::<Vec<_>>(),
        [word('a'), word('b')]
    );
    assert_eq!(
        arena.words(region, 1).iter().collect::<Vec<_>>(),
        [word('c'), word('x')]
    );
    assert_eq!(
        arena
            .values(region)
            .iter()
            .map(ArgumentValue::len)
            .collect::<Vec<_>>(),
        [2, 2, 0]
    );
    arena.release(region);
    assert!(arena.is_empty());
}

#[test]
fn trimmed_match_iterator_stops_at_both_stored_range_boundaries() {
    let mut arena = ArgumentArena::<()>::default();
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
        let mut arena = ArgumentArena::<()>::default();
        let matching = arena.begin_match().expect("macro match");
        let mut writer = arena.begin_argument(&matching);
        append_all(&mut arena, &mut writer, words);
        assert_eq!(writer.removable_outer_group(), expected);
    }
}

#[test]
fn fused_forwarding_preserves_refused_words_on_either_side_of_the_destination() {
    for lower_destination in [false, true] {
        for count in [1, 8192] {
            let mut arena = ArgumentArena::<()>::default();
            let reserved = if lower_destination {
                let matching = arena.begin_match().expect("reserve lower region");
                Some(arena.publish(matching))
            } else {
                None
            };
            let mut parent = arena.begin_match().expect("parent match");
            let mut cursor = arena.begin_argument(&parent);
            let words = vec![word('p'); count]
                .into_iter()
                .chain([
                    brace('{', Catcode::BeginGroup),
                    word('q'),
                    brace('}', Catcode::EndGroup),
                ])
                .collect::<Vec<_>>();
            append_all(&mut arena, &mut cursor, words.iter().copied());
            arena
                .publish_argument(&mut parent, cursor)
                .expect("parent span");
            let parent = arena.publish(parent);
            if let Some(reserved) = reserved {
                arena.release(reserved);
            }
            let child = arena.begin_match().expect("child match");
            let mut cursor = arena.begin_argument(&child);
            let accepted = cursor
                .consume_region_span(
                    &mut arena,
                    parent,
                    0,
                    0..words.len(),
                    &mut crate::execution_scratch::ArgumentRunAdmission::characters(),
                )
                .expect("forward admitted prefix");
            assert_eq!(accepted, count);
            assert_eq!(
                arena.words(parent, 0).iter().collect::<Vec<_>>(),
                words,
                "refused group stays in the source"
            );
            assert_eq!(
                arena
                    .match_words(&cursor)
                    .expect("child words")
                    .collect::<Vec<_>>(),
                &words[..count]
            );
            assert_eq!(cursor.brace_depth(), 0);
            assert!(!cursor.removable_outer_group());
            arena.release(parent);
            assert_eq!(
                arena
                    .match_words(&cursor)
                    .expect("independent child")
                    .count(),
                count
            );
            arena.abort_capture();
        }
    }
}

#[test]
fn forwarded_capture_survives_parent_release_and_abort_preserves_live_parent() {
    let mut arena = ArgumentArena::<()>::default();
    let mut parent = arena.begin_match().expect("parent match");
    let mut cursor = arena.begin_argument(&parent);
    // A long argument remains valid after its source invocation retires.
    let words = vec![word('p'); 8_192];
    append_all(&mut arena, &mut cursor, words.iter().copied());
    arena
        .publish_argument(&mut parent, cursor)
        .expect("parent span");
    let parent = arena.publish(parent);
    let mut child = arena.begin_match().expect("child match");
    let mut cursor = arena.begin_argument(&child);
    let count = cursor
        .consume_region_span(
            &mut arena,
            parent,
            0,
            0..words.len(),
            &mut crate::execution_scratch::ArgumentRunAdmission::characters(),
        )
        .expect("forward final words");
    assert_eq!(count, words.len());
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
    let child = arena.publish(child);
    let failed = arena.begin_match().expect("next nested match");
    let mut cursor = arena.begin_argument(&failed);
    append_all(&mut arena, &mut cursor, [word('x')]);
    arena.abort_capture();
    assert_eq!(
        arena.words(child, 0).iter().collect::<Vec<_>>(),
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

#[test]
fn reused_argument_region_does_not_retain_prior_range_suffix_as_semantic_state() {
    fn capture(arena: &mut ArgumentArena<()>, arguments: &[char]) -> ArgumentRegionId {
        let mut matching = arena.begin_match().expect("match owner");
        for &ch in arguments {
            let mut cursor = arena.begin_argument(&matching);
            append_all(arena, &mut cursor, [word(ch)]);
            arena
                .publish_argument(&mut matching, cursor)
                .expect("argument range");
        }
        arena.publish(matching)
    }
    let mut reused = ArgumentArena::<()>::default();
    let prior = capture(&mut reused, &['a', 'b', 'c']);
    reused.release(prior);
    let current = capture(&mut reused, &['x']);
    let mut fresh = ArgumentArena::<()>::default();
    let other = capture(&mut fresh, &['x']);
    assert!(
        reused
            .words(current, 0)
            .iter()
            .eq(fresh.words(other, 0).iter())
    );
    assert_eq!(reused.values(current)[0].len(), 1);
    assert_eq!(
        reused, fresh,
        "unused prior ranges and pooled capacity are not semantic state"
    );
    fn semantic_hash(arena: &ArgumentArena<()>) -> u64 {
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        arena.hash(&mut hash);
        hash.finish()
    }
    assert_eq!(semantic_hash(&reused), semantic_hash(&fresh));
}

#[test]
fn fragmented_argument_materializes_before_either_source_retires() {
    let mut arena = ArgumentArena::<()>::default();
    let mut sources = Vec::new();
    for ch in ['a', 'b'] {
        let mut matching = arena.begin_match().expect("source match");
        let mut cursor = arena.begin_argument(&matching);
        append_all(&mut arena, &mut cursor, [word(ch)]);
        arena
            .publish_argument(&mut matching, cursor)
            .expect("source argument");
        sources.push(arena.publish(matching));
    }
    let mut child = arena.begin_match().expect("fragmented match");
    let mut cursor = arena.begin_argument(&child);
    for source in sources {
        assert_eq!(
            cursor.consume_region_span(
                &mut arena,
                source,
                0,
                0..1,
                &mut ArgumentRunAdmission::characters(),
            ),
            Ok(1)
        );
        arena.release(source);
    }
    arena
        .publish_argument(&mut child, cursor)
        .expect("fragmented argument");
    let child = arena.publish(child);
    assert_eq!(
        arena.words(child, 0).iter().collect::<Vec<_>>(),
        [word('a'), word('b')]
    );
    arena.release(child);
    assert!(arena.is_empty());
}

#[test]
fn definition_argument_survives_publication_and_source_body_retirement() {
    crate::test_harness::with_universe(|universe| {
        let publication = universe
            .allocate_definition(
                &[tex_state::token::TokenWord::pack(Token::Param(1))],
                &[
                    word('a').token_word(),
                    word('b').token_word(),
                    word('c').token_word(),
                ],
            )
            .expect("source definition");
        let body = universe
            .command_context()
            .expect("context")
            .admit_macro_body(publication.reference())
            .expect("resident body");
        let mut arena = ArgumentArena::default();
        let mut matching = arena.begin_match().expect("argument match");
        let mut cursor = arena.begin_argument(&matching);
        assert_eq!(
            cursor.consume_token_span(
                &mut arena,
                &body,
                1..3,
                &mut ArgumentRunAdmission::characters(),
            ),
            Ok(2)
        );
        drop(body);
        drop(publication);
        arena
            .publish_argument(&mut matching, cursor)
            .expect("argument");
        let region = arena.publish(matching);
        assert_eq!(
            arena.words(region, 0).iter().collect::<Vec<_>>(),
            [word('b'), word('c')]
        );
        arena.release(region);
        assert!(arena.is_empty());
    });
}

#[test]
fn deeply_nested_argument_leases_release_without_recursive_drops() {
    let mut arena = ArgumentArena::<()>::default();
    let mut previous = None;
    for _ in 0..10_000 {
        let mut matching = arena.begin_match().expect("nested match");
        let mut own = arena.begin_argument(&matching);
        append_all(&mut arena, &mut own, [word('x')]);
        arena
            .publish_argument(&mut matching, own)
            .expect("own argument");
        if let Some(source) = previous {
            let mut borrowed = arena.begin_argument(&matching);
            assert_eq!(
                borrowed.consume_region_span(
                    &mut arena,
                    source,
                    0,
                    0..1,
                    &mut ArgumentRunAdmission::characters(),
                ),
                Ok(1)
            );
            arena
                .publish_argument(&mut matching, borrowed)
                .expect("borrowed argument");
            arena.release(source);
        }
        previous = Some(arena.publish(matching));
    }
    let last = previous.expect("last invocation");
    assert_eq!(arena.words(last, 1).get(0), Some(word('x')));
    arena.release(last);
    assert!(arena.is_empty());
    let matching = arena.begin_match().expect("reuse released region");
    let mut cursor = arena.begin_argument(&matching);
    append_all(&mut arena, &mut cursor, [word('y')]);
    assert_eq!(
        arena
            .match_words(&cursor)
            .expect("reused words")
            .collect::<Vec<_>>(),
        [word('y')]
    );
    arena.abort_capture();
    assert!(arena.is_empty());
}
