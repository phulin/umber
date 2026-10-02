use super::{AttemptArena, AttemptError};
use tex_state::interner::InternerBudget;
use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};
fn word(ch: char) -> TracedTokenWord {
    TracedTokenWord::pack(
        Token::Char {
            ch,
            cat: Catcode::Other,
        },
        OriginId::UNKNOWN,
    )
}
fn budget() -> InternerBudget {
    InternerBudget::new(64, 64, 4096).expect("test budget")
}

#[test]
fn scanner_destinations_share_fixed_chunks_and_are_mark_bounded() {
    tex_state::with_universe(budget(), |_universe| {
        let mut attempt = AttemptArena::<()>::default();
        let mark = attempt.mark();
        let buffer = attempt
            .allocate_token_buffer()
            .expect("test fixture is valid");
        attempt
            .push_buffer_token(&buffer, word('a'))
            .expect("test fixture is valid");
        attempt
            .push_buffer_token(&buffer, word('b'))
            .expect("test fixture is valid");
        assert_eq!(
            attempt
                .token_buffer(&buffer)
                .expect("test fixture is valid"),
            &[word('a'), word('b')]
        );
        let retained_chunks = attempt.token_lane.retained_chunks();
        let frozen = attempt
            .finish_token_buffer(buffer)
            .expect("test fixture is valid");
        assert_eq!(
            attempt.token_words(&frozen).expect("test fixture is valid"),
            &[word('a'), word('b')]
        );
        assert_eq!(attempt.token_lane.retained_chunks(), retained_chunks);

        attempt.truncate(mark).expect("test fixture is valid");
        let recycled = attempt
            .allocate_token_buffer()
            .expect("test fixture is valid");
        assert_eq!(
            attempt.token_lane.retained_chunks(),
            retained_chunks,
            "retiring the scanner result returns its chunks to the shared attempt lane"
        );
        assert_eq!(
            attempt.token_buffer(&buffer),
            Err(AttemptError::InvalidCoordinate)
        );
        assert!(
            attempt
                .token_buffer(&recycled)
                .expect("recycled sink")
                .is_empty()
        );
        assert_eq!(
            attempt.token_words(&frozen),
            Err(AttemptError::InvalidCoordinate)
        );
        attempt.truncate(mark).expect("recycled buffer retires");
        assert_eq!(
            attempt.token_buffer(&recycled),
            Err(AttemptError::InvalidCoordinate)
        );
    })
    .expect("test fixture is valid");
}

#[test]
fn nested_scanner_suffix_truncation_preserves_the_parent_destination() {
    let mut attempt = AttemptArena::<()>::default();
    let outer = attempt
        .allocate_token_buffer()
        .expect("outer scanner destination");
    for _ in 0..65 {
        attempt
            .push_buffer_token(&outer, word('a'))
            .expect("outer scanner word");
    }

    let child_opening = attempt.mark();
    let child = attempt
        .allocate_token_buffer()
        .expect("child scanner destination");
    for _ in 0..65 {
        attempt
            .push_buffer_token(&child, word('x'))
            .expect("child scanner word");
    }
    attempt
        .truncate(child_opening)
        .expect("child scanner suffix truncates");

    attempt
        .push_buffer_token(&outer, word('b'))
        .expect("outer destination continues in place");
    let outer = attempt
        .finish_token_buffer(outer)
        .expect("outer scanner result");
    let words = attempt.token_words(&outer).expect("outer scanner words");
    assert_eq!(words.len(), 66);
    assert!(words.iter().take(65).all(|entry| *entry == word('a')));
    assert_eq!(words.get(65).copied(), Some(word('b')));
    assert_eq!(
        attempt.token_buffer(&child),
        Err(AttemptError::InvalidCoordinate)
    );
    assert_eq!(attempt.token_lane.retained_chunks(), 4);
}

#[cfg(feature = "profiling")]
#[test]
fn one_and_4096_child_scanner_splices_transfer_ownership_without_copy() {
    for token_count in [1, 4_096] {
        let mut attempt = AttemptArena::<()>::default();
        let parent = attempt
            .allocate_token_buffer()
            .expect("parent scanner destination");
        attempt
            .push_buffer_token(&parent, word('p'))
            .expect("parent prefix");
        let child = attempt
            .allocate_token_buffer()
            .expect("child scanner destination");
        for _ in 0..token_count {
            attempt
                .push_buffer_token(&child, word('c'))
                .expect("child scanner word");
        }
        let child = attempt
            .finish_token_buffer(child)
            .expect("completed child list");
        let lane_before = attempt.token_lane.counters();
        let owner = tex_state::measurement::HotCoreAllocationOwner::AttemptScratch;
        let allocations_before =
            tex_state::measurement::hot_core_thread_allocation_measurement(owner);
        let moved = {
            let _scope = tex_state::measurement::hot_core_allocation_scope(owner);
            attempt
                .consume_token_list_into_buffer(child.test_alias(), &parent)
                .expect("child chain transfers to parent")
        };
        let allocations_after =
            tex_state::measurement::hot_core_thread_allocation_measurement(owner);
        let lane_after = attempt.token_lane.counters();

        assert_eq!(moved, token_count);
        assert_eq!(lane_after.words_appended, lane_before.words_appended);
        assert_eq!(lane_after.chunk_allocations, lane_before.chunk_allocations);
        assert_eq!(allocations_after.calls - allocations_before.calls, 0);
        assert_eq!(
            allocations_after.requested_bytes - allocations_before.requested_bytes,
            0
        );
        assert_eq!(
            attempt.token_words(&child),
            Err(AttemptError::InvalidCoordinate)
        );
        let parent = attempt
            .finish_token_buffer(parent)
            .expect("completed parent list");
        let words = attempt
            .token_words(&parent)
            .expect("transferred parent words");
        assert_eq!(words.len(), token_count as usize + 1);
        assert_eq!(words.first().copied(), Some(word('p')));
        assert!(words.iter().skip(1).all(|entry| *entry == word('c')));
    }
}

#[cfg(feature = "profiling")]
#[test]
fn one_and_4096_parent_owned_scans_reuse_chunks_without_allocation_or_transfer() {
    fn run(attempt: &mut AttemptArena<()>) {
        let mark = attempt.mark();
        let buffer = attempt.allocate_token_buffer().expect("scanner buffer");
        for _ in 0..65 {
            attempt
                .push_buffer_token(&buffer, word('x'))
                .expect("scanner word");
        }
        let before_finish = attempt.token_lane.counters();
        let result = attempt.finish_token_buffer(buffer).expect("scanner result");
        assert_eq!(
            attempt.token_lane.counters(),
            before_finish,
            "finalization only publishes the existing sink coordinate"
        );
        let words = attempt.token_words(&result).expect("scanner words");
        assert_eq!(words.len(), 65);
        assert_eq!(words.first().copied(), Some(word('x')));
        assert_eq!(words.get(64).copied(), Some(word('x')));
        attempt.truncate(mark).expect("scanner scope retires");
    }

    fn evidence(scans: u64) {
        let mut attempt = AttemptArena::<()>::default();
        run(&mut attempt);
        assert_eq!(attempt.token_lane.retained_chunks(), 2);
        let counters_before = attempt.token_lane.counters();
        let owner = tex_state::measurement::HotCoreAllocationOwner::AttemptScratch;
        let allocations_before =
            tex_state::measurement::hot_core_thread_allocation_measurement(owner);
        for _ in 0..scans {
            run(&mut attempt);
        }
        let allocations_after =
            tex_state::measurement::hot_core_thread_allocation_measurement(owner);
        let counters_after = attempt.token_lane.counters();
        assert_eq!(
            counters_after.chunk_allocations - counters_before.chunk_allocations,
            0
        );
        assert_eq!(
            counters_after.chunk_reuses - counters_before.chunk_reuses,
            scans * 2
        );
        assert_eq!(
            counters_after.words_appended - counters_before.words_appended,
            scans * 65
        );
        assert_eq!(
            counters_after.chunks_released - counters_before.chunks_released,
            scans * 2
        );
        assert_eq!(allocations_after.calls - allocations_before.calls, 0);
        assert_eq!(
            allocations_after.requested_bytes - allocations_before.requested_bytes,
            0
        );
        assert_eq!(attempt.token_lane.retained_chunks(), 2);
    }

    for scans in [1, 4_096] {
        evidence(scans);
    }
}

#[test]
fn truncated_row_cannot_alias_a_reallocated_coordinate() {
    tex_state::with_universe(budget(), |_universe| {
        let mut attempt = AttemptArena::<()>::default();
        let mark = attempt.mark();
        let stale = attempt
            .allocate_token_list([word('a')])
            .expect("test fixture is valid");
        attempt.truncate(mark).expect("test fixture is valid");
        let replacement = attempt
            .allocate_token_list([word('b')])
            .expect("test fixture is valid");

        assert_eq!(
            attempt.token_words(&stale),
            Err(AttemptError::InvalidCoordinate)
        );
        assert_eq!(
            attempt
                .token_words(&replacement)
                .expect("test fixture is valid"),
            &[word('b')]
        );
    })
    .expect("test fixture is valid");
}

#[test]
fn child_scopes_close_exact_lifo_suffixes() {
    let mut attempt = AttemptArena::<()>::default();
    let retained = attempt
        .allocate_token_list([word('p')])
        .expect("parent value");
    let parent = attempt.begin_child_scope();
    let parent_value = attempt
        .allocate_token_list([word('a')])
        .expect("parent-scope value");
    let child = attempt.begin_child_scope();
    let child_value = attempt
        .allocate_token_list([word('b')])
        .expect("child-scope value");

    attempt
        .close_child_scope(child)
        .expect("child closes first");
    assert_eq!(
        attempt.token_words(&child_value),
        Err(AttemptError::InvalidCoordinate)
    );
    assert_eq!(
        attempt.token_words(&parent_value).expect("parent words"),
        &[word('a')]
    );
    attempt
        .close_child_scope(parent)
        .expect("parent closes second");
    assert_eq!(
        attempt.token_words(&retained).expect("retained words"),
        &[word('p')]
    );
}

#[test]
fn sealed_or_consumed_rows_cannot_be_written_through_the_old_buffer_alias() {
    let mut arena = AttemptArena::<()>::default();
    let child = arena.allocate_token_buffer().expect("child");
    arena
        .push_buffer_token(&child, word('x'))
        .expect("child word");
    let list = arena.finish_token_buffer(child).expect("seal child");
    assert_eq!(
        arena.push_buffer_token(&child, word('y')),
        Err(AttemptError::InvalidCoordinate)
    );
    assert_eq!(
        arena.finish_token_buffer(child),
        Err(AttemptError::InvalidCoordinate)
    );
    let parent = arena.allocate_token_buffer().expect("parent");
    arena
        .consume_token_list_into_buffer(list.test_alias(), &parent)
        .expect("transfer child");
    assert_eq!(
        arena.token_words(&list),
        Err(AttemptError::InvalidCoordinate)
    );
    assert_eq!(
        arena.token_buffer(&child),
        Err(AttemptError::InvalidCoordinate)
    );
    assert_eq!(
        arena.consume_token_list_into_buffer(list.test_alias(), &parent),
        Err(AttemptError::InvalidCoordinate)
    );
    assert_eq!(
        arena.token_buffer(&parent).expect("parent words"),
        &[word('x')]
    );
}

#[test]
fn serial_exhaustion_and_foreign_marks_do_not_publish_rows() {
    let mut arena = AttemptArena::<()>::default();
    let other = AttemptArena::<()>::default();
    let before = arena.mark();
    assert_eq!(
        arena.truncate(other.mark()),
        Err(AttemptError::ForeignAttempt)
    );
    arena.next_serial = u64::MAX;
    assert_eq!(
        arena.allocate_token_buffer(),
        Err(AttemptError::CapacityOverflow)
    );
    assert!(arena.is_clear());
    let _ = before;
    assert!(arena.is_clear());
}

#[test]
fn consumers_retire_rows_without_a_command_boundary() {
    let mut arena = AttemptArena::<()>::default();
    let old = arena.allocate_token_list([word('a')]).expect("list");
    let alias = old.test_alias();
    let child = arena.mark();
    let open = arena.allocate_token_buffer().expect("open sink");
    arena
        .push_buffer_token(&open, word('b'))
        .expect("open word");
    arena
        .discard_token_list(old)
        .expect("consumer releases root below child");
    assert_eq!(
        arena.token_words(&alias),
        Err(AttemptError::InvalidCoordinate)
    );
    arena
        .truncate(child)
        .expect("failed child discards its suffix");
    assert!(arena.is_clear());
    let new = arena.allocate_token_list([word('c')]).expect("new list");
    assert_eq!(arena.token_words(&new).expect("new words"), &[word('c')]);
    arena.discard_token_list(new).expect("new consumer");
    assert!(arena.is_clear());
}

#[test]
fn consumed_parent_does_not_invalidate_a_child_serial_frontier() {
    let mut arena = AttemptArena::<()>::default();
    let parent = arena.allocate_token_list([word('p')]).expect("parent");
    let child = arena.begin_child_scope();
    arena.discard_token_list(parent).expect("parent consumer");
    let child_value = arena.allocate_token_list([word('c')]).expect("child value");
    arena
        .discard_token_list(child_value)
        .expect("child consumer");
    arena
        .close_child_scope(child)
        .expect("serial frontier survives recycled rows");
    assert!(arena.is_clear());
}
