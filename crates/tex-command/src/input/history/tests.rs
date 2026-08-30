use tex_state::token::{Catcode, OriginId, Token, TokenWord, TracedTokenWord};

#[cfg(feature = "profiling")]
use tex_state::measurement::HotCoreAllocationOwner;

use super::InputStack;
use crate::input::{
    InputLevel, InputLevelId, MacroArgumentCursor, PackedTokenSources, PackedTokenSpanHandle,
    PackedTokenSpanSource, ReplayLane, ReplayTrace, RetirementBehavior, TokenBehavior, TokenCursor,
    packed_token_frame,
};

fn word(ch: char) -> TracedTokenWord {
    TracedTokenWord::from_parts(
        TokenWord::pack(Token::Char {
            ch,
            cat: Catcode::Other,
        }),
        OriginId::UNKNOWN,
    )
}

fn assert_exact_direct_transition<G>(
    stack: &mut InputStack<G>,
    mutate: impl Fn(&mut InputStack<G>),
) {
    let checkpoint = stack.mark().expect("input checkpoint");
    stack.reset_cursor_mutation_counters();
    let opening_revision = stack.context_revision();
    let before_history = stack.counters();
    #[cfg(feature = "profiling")]
    let owner = HotCoreAllocationOwner::DeliveryAndScan;
    #[cfg(feature = "profiling")]
    let before_allocations = tex_state::measurement::hot_core_thread_allocation_measurement(owner);
    {
        #[cfg(feature = "profiling")]
        let _scope = tex_state::measurement::hot_core_allocation_scope(owner);
        mutate(stack);
        mutate(stack);
    }
    #[cfg(feature = "profiling")]
    let after_allocations = tex_state::measurement::hot_core_thread_allocation_measurement(owner);
    let after_history = stack.counters();

    assert_eq!(
        stack.cursor_mutation_counters(),
        super::InputCursorMutationCounters {
            typed_top_accesses: 2,
            first_touch_transitions: 1,
            coalesced_transitions: 1,
            closure_dispatches: 0,
        }
    );
    assert_eq!(after_history.undo_records - before_history.undo_records, 1);
    assert_eq!(
        after_history.coalesced_mutations - before_history.coalesced_mutations,
        1
    );
    assert_eq!(
        stack.context_revision(),
        opening_revision.wrapping_add(2).max(1)
    );
    assert_eq!(stack.as_slice().last().map(cursor_position), Some(2));

    stack.begin_checkpoint_candidate(checkpoint);
    assert_eq!(stack.as_slice().last().map(cursor_position), Some(0));
    stack.reject_checkpoint_candidate();
    assert_eq!(stack.as_slice().last().map(cursor_position), Some(2));
    #[cfg(feature = "profiling")]
    {
        assert_eq!(after_allocations.calls - before_allocations.calls, 0);
        assert_eq!(
            after_allocations.requested_bytes - before_allocations.requested_bytes,
            0
        );
    }
}

fn cursor_position<G>(level: &InputLevel<G>) -> usize {
    match level {
        InputLevel::Tokens(cursor) => cursor.position(),
        InputLevel::MacroArgument(cursor) => cursor.position(),
        InputLevel::Source(_) => panic!("fixture admits a stored-token cursor"),
    }
}

#[test]
fn token_cursor_mutation_is_one_typed_access_and_one_coalesced_journal_transition() {
    let behavior = TokenBehavior::Ordinary;
    let retirement = RetirementBehavior::Pop;
    let trace = ReplayTrace::Inserted;
    let mut replay = ReplayLane::default();
    let span = PackedTokenSpanHandle::transient([word('a'), word('b')])
        .admit(&mut replay)
        .expect("token span admits");
    let attempt = crate::attempt::AttemptArena::default();
    let scratch = crate::execution_scratch::ExecutionScratch::default();
    let mut stack = InputStack::<()>::default();
    stack.push(InputLevel::Tokens(TokenCursor {
        span,
        frame: packed_token_frame(InputLevelId(1), 2, &behavior, retirement, &trace),
        behavior,
        retirement,
        trace,
    }));

    assert_exact_direct_transition(&mut stack, |stack| {
        let mut command = crate::command::CurrentCommand::empty();
        let delivery = stack
            .deliver_top_cursor_into(
                PackedTokenSources::new(&replay, &attempt),
                &scratch,
                command.empty_for_raw_delivery(),
            )
            .expect("token cursor remains on top")
            .expect("token delivery succeeds");
        assert!(delivery.raw.is_some());
    });
}

#[test]
fn macro_argument_mutation_uses_the_same_direct_transition() {
    let mut scratch = crate::execution_scratch::ExecutionScratch::default();
    let matching = scratch.begin_macro_match().expect("macro match");
    let mut buffer = scratch.begin_match_buffer(&matching).expect("match buffer");
    for spelling in [word('a'), word('b')] {
        scratch
            .push_match_word(
                &mut buffer,
                spelling,
                crate::execution_scratch::MacroArgumentTokenFacts::default(),
            )
            .expect("argument word");
    }
    scratch.finish_match_buffer(buffer).expect("argument range");
    let macro_frame = scratch
        .commit_macro_match(matching)
        .expect("sealed macro frame");
    let range = scratch
        .argument_range(macro_frame, 1)
        .expect("live macro frame")
        .expect("first argument");
    let behavior = TokenBehavior::Parameter;
    let retirement = RetirementBehavior::Pop;
    let trace = ReplayTrace::MacroParameter { slot: 1 };
    let mut stack = InputStack::<()>::default();
    stack.push(InputLevel::MacroArgument(MacroArgumentCursor {
        range,
        slot: 1,
        frame: packed_token_frame(InputLevelId(2), 2, &behavior, retirement, &trace),
    }));
    let replay = ReplayLane::default();
    let attempt = crate::attempt::AttemptArena::default();

    assert_exact_direct_transition(&mut stack, |stack| {
        let mut command = crate::command::CurrentCommand::empty();
        let delivery = stack
            .deliver_top_cursor_into(
                PackedTokenSources::new(&replay, &attempt),
                &scratch,
                command.empty_for_raw_delivery(),
            )
            .expect("macro argument remains on top")
            .expect("macro-argument delivery succeeds");
        assert!(delivery.raw.is_some());
    });
}
