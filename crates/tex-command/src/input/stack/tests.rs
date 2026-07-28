use std::sync::{Arc, Weak};

use tex_state::Universe;
use tex_state::ids::{MacroDefinitionId, OriginListId, TokenListId};
use tex_state::meaning::Meaning;
use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};

use crate::macro_call::{MacroActivation, MacroActivationId, MacroArgumentRange, MacroArguments};
use crate::{CommandState, RegisteredSourceKind, SourceRegistration};

use super::{
    InputRetirementAction, InputRetirementError, InputRetirementReason, OutParameterReplay,
    ParameterReplayError,
};
use crate::input::levels::{
    BackupTreatment, InputLevel, ReplayTrace, RetirementBehavior, SharedBackedUpBuffer,
    SharedTokenBuffer, StoredReplayReason, TokenBehavior, TokenPayload, TransientReplayReason,
};

fn traced(ch: char) -> TracedTokenWord {
    TracedTokenWord::pack(
        Token::Char {
            ch,
            cat: Catcode::Other,
        },
        OriginId::UNKNOWN,
    )
}

fn transient_payload(tokens: &[TracedTokenWord]) -> TokenPayload {
    TokenPayload::Transient(SharedTokenBuffer::new(tokens.to_vec()))
}

fn push_activation(
    state: &mut CommandState,
    identity: u64,
    tokens: Arc<[TracedTokenWord]>,
    ranges: [Option<MacroArgumentRange>; 9],
) -> MacroActivationId {
    let identity = MacroActivationId(identity);
    state.parameters.activations.push(MacroActivation {
        identity,
        definition: MacroDefinitionId::testing_new(
            u32::try_from(identity.0 + 100).expect("test definition identity fits"),
        ),
        arguments: MacroArguments {
            buffer: SharedTokenBuffer::new(tokens),
            ranges,
        },
        invocation: OriginId::UNKNOWN,
    });
    identity
}

#[test]
fn each_popped_level_retires_exactly_once_with_its_trace() {
    let mut state = CommandState::default();
    let identity = state.push_token_level(
        TokenPayload::Stored {
            tokens: TokenListId::EMPTY,
            origins: OriginListId::EMPTY,
        },
        TokenBehavior::Ordinary,
        RetirementBehavior::StopAtEnd,
        ReplayTrace::Stored(StoredReplayReason::EveryJob),
    );

    let retirement = state
        .retire_exhausted_input(identity)
        .expect("the exact exhausted level retires");
    assert_eq!(retirement.identity, identity);
    assert_eq!(retirement.action, InputRetirementAction::TerminalStop);
    assert_eq!(
        retirement.trace,
        Some(ReplayTrace::Stored(StoredReplayReason::EveryJob))
    );
    assert_eq!(
        state.retire_exhausted_input(identity),
        Err(InputRetirementError::NoInput)
    );
}

#[test]
fn retirement_validates_exact_level_identity_before_mutating() {
    let mut state = CommandState::default();
    let first = state.push_token_level(
        transient_payload(&[]),
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::Inserted,
    );
    let second = state.push_token_level(
        transient_payload(&[]),
        TokenBehavior::Ordinary,
        RetirementBehavior::CloseScantokens,
        ReplayTrace::Transient(TransientReplayReason::Scantokens),
    );

    assert_eq!(
        state.retire_exhausted_input(first),
        Err(InputRetirementError::LevelChanged {
            expected: first,
            actual: second,
        })
    );
    assert_eq!(state.input.levels.len(), 2);
    assert_eq!(
        state
            .retire_exhausted_input(second)
            .expect("current level retires")
            .action,
        InputRetirementAction::ScantokensClosed
    );
}

#[test]
fn exhausted_v_template_reports_its_boundary_before_the_next_fetch_pops_it() {
    let mut state = CommandState::default();
    let identity = state.push_token_level(
        transient_payload(&[]),
        TokenBehavior::VTemplate,
        RetirementBehavior::RetainExhaustedVTemplate,
        ReplayTrace::VTemplate,
    );

    // tex.web §§325/390 refuse to drain a v-template for stack conservation,
    // and §1131's `do_endv` expects to find this frame live, so the first
    // exhaustion only reports the frozen `end_template` boundary.
    assert_eq!(
        state
            .retire_exhausted_input(identity)
            .expect("exhaustion retains the template")
            .action,
        InputRetirementAction::VTemplateRetained
    );
    assert_eq!(state.input.levels.len(), 1);
    // tex.web §357: once that boundary is reported the frame is an ordinary
    // depleted token list, so the next `get_next` that reaches it runs
    // `end_token_list`.  Nothing at the `do_endv` call site pops it.
    assert_eq!(
        state
            .retire_exhausted_input(identity)
            .expect("the next fetch pops the depleted template")
            .action,
        InputRetirementAction::VTemplatePopped
    );
    assert!(state.input.levels.is_empty());
}

#[test]
fn transient_payload_drops_with_its_last_input_owner() {
    let allocation: Arc<[TracedTokenWord]> = vec![traced('x')].into();
    let weak: Weak<[TracedTokenWord]> = Arc::downgrade(&allocation);
    let mut state = CommandState::default();
    let identity = state.push_token_level(
        TokenPayload::Transient(SharedTokenBuffer::new(Arc::clone(&allocation))),
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::Inserted,
    );
    drop(allocation);
    assert!(weak.upgrade().is_some());

    state
        .retire_exhausted_input(identity)
        .expect("transient insertion retires");
    assert!(weak.upgrade().is_none());
}

#[test]
fn snapshot_ownership_keeps_transient_payload_live_past_stack_retirement() {
    let allocation: Arc<[TracedTokenWord]> = vec![traced('x')].into();
    let weak: Weak<[TracedTokenWord]> = Arc::downgrade(&allocation);
    let mut state = CommandState::default();
    let identity = state.push_token_level(
        TokenPayload::Transient(SharedTokenBuffer::new(Arc::clone(&allocation))),
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::Inserted,
    );
    let snapshot = state.snapshot();
    drop(allocation);

    state
        .retire_exhausted_input(identity)
        .expect("live cursor retires");
    assert!(weak.upgrade().is_some());
    drop(snapshot);
    assert!(weak.upgrade().is_none());
}

#[test]
fn parameter_replay_uses_nearest_macro_body_param_start() {
    let outer_tokens: Arc<[TracedTokenWord]> = vec![traced('o')].into();
    let inner_tokens: Arc<[TracedTokenWord]> = vec![traced('i'), traced('n')].into();
    let mut state = CommandState::default();
    let outer = push_activation(
        &mut state,
        1,
        outer_tokens,
        [
            MacroArgumentRange::new(0, 1),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        ],
    );
    state.push_token_level(
        transient_payload(&[traced('A')]),
        TokenBehavior::MacroBody(outer),
        RetirementBehavior::Pop,
        ReplayTrace::MacroReplacement,
    );
    let inner = push_activation(
        &mut state,
        2,
        inner_tokens,
        [
            MacroArgumentRange::new(0, 1),
            MacroArgumentRange::new(1, 2),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        ],
    );
    state.push_token_level(
        transient_payload(&[traced('B')]),
        TokenBehavior::MacroBody(inner),
        RetirementBehavior::Pop,
        ReplayTrace::MacroReplacement,
    );
    let nested = state.push_token_level(
        transient_payload(&[traced('#')]),
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::Transient(TransientReplayReason::ExpandedTokenList),
    );

    let OutParameterReplay::Pushed(parameter) = state
        .replay_out_parameter(nested, 2)
        .expect("nested replay resolves through the nearest param_start")
    else {
        panic!("the parameter range was not pushed");
    };
    let crate::input::InputLevel::Tokens(cursor) =
        state.input.levels.last().expect("parameter level")
    else {
        panic!("parameter replay was not a token level");
    };
    assert_eq!(cursor.identity, parameter);
    assert_eq!(cursor.behavior, TokenBehavior::Parameter);
    assert_eq!(cursor.trace, ReplayTrace::MacroParameter { slot: 2 });
    let TokenPayload::ArgumentRange { buffer, range } = &cursor.payload else {
        panic!("parameter replay did not share its activation buffer");
    };
    assert_eq!(buffer.len(), 2);
    assert_eq!((range.start(), range.end()), (1, 2));
}

#[test]
fn parameter_level_replays_out_parameter_tokens_literally() {
    let mut state = CommandState::default();
    let identity = state.push_token_level(
        transient_payload(&[traced('#')]),
        TokenBehavior::Parameter,
        RetirementBehavior::Pop,
        ReplayTrace::MacroParameter { slot: 1 },
    );

    assert_eq!(
        state.replay_out_parameter(identity, 1),
        Ok(OutParameterReplay::Literal)
    );
    assert_eq!(state.input.levels.len(), 1);
}

#[test]
fn activation_records_without_a_live_param_start_do_not_own_replay() {
    let mut state = CommandState::default();
    push_activation(
        &mut state,
        3,
        vec![traced('x')].into(),
        [
            MacroArgumentRange::new(0, 1),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        ],
    );
    let nested = state.push_token_level(
        transient_payload(&[traced('#')]),
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::Inserted,
    );

    assert_eq!(
        state.replay_out_parameter(nested, 1),
        Err(ParameterReplayError::NoMacroOwner)
    );
}

#[test]
fn source_input_is_a_param_start_ownership_boundary() {
    let mut state = CommandState::default();
    let owner = push_activation(
        &mut state,
        4,
        vec![traced('x')].into(),
        [
            MacroArgumentRange::new(0, 1),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        ],
    );
    state.push_token_level(
        transient_payload(&[]),
        TokenBehavior::MacroBody(owner),
        RetirementBehavior::Pop,
        ReplayTrace::MacroReplacement,
    );
    let source = state
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Vec::new(),
        ))
        .expect("empty exact-byte source is valid");
    state
        .open_registered_source(source)
        .expect("registered source opens");
    let nested = state.push_token_level(
        transient_payload(&[traced('#')]),
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::Inserted,
    );

    assert_eq!(
        state.replay_out_parameter(nested, 1),
        Err(ParameterReplayError::NoMacroOwner)
    );
}

#[test]
fn macro_body_retirement_releases_exactly_its_activation_and_arguments() {
    let allocation: Arc<[TracedTokenWord]> = vec![traced('x')].into();
    let weak = Arc::downgrade(&allocation);
    let mut state = CommandState::default();
    let activation = push_activation(
        &mut state,
        9,
        Arc::clone(&allocation),
        [
            MacroArgumentRange::new(0, 1),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        ],
    );
    let body = state.push_token_level(
        transient_payload(&[]),
        TokenBehavior::MacroBody(activation),
        RetirementBehavior::Pop,
        ReplayTrace::MacroReplacement,
    );
    drop(allocation);

    state
        .retire_exhausted_input(body)
        .expect("macro body and param_start retire together");
    assert!(state.parameters.activations.is_empty());
    assert!(weak.upgrade().is_none());
}

#[test]
fn macro_body_retirement_rejects_activation_mismatch_transactionally() {
    let mut state = CommandState::default();
    let actual = push_activation(
        &mut state,
        10,
        Arc::from([]),
        [None, None, None, None, None, None, None, None, None],
    );
    let expected = MacroActivationId(11);
    let body = state.push_token_level(
        transient_payload(&[]),
        TokenBehavior::MacroBody(expected),
        RetirementBehavior::Pop,
        ReplayTrace::MacroReplacement,
    );

    assert_eq!(
        state.retire_exhausted_input(body),
        Err(InputRetirementError::MacroActivationOrder {
            expected,
            actual: Some(actual),
        })
    );
    assert_eq!(state.input.levels.len(), 1);
    assert_eq!(state.parameters.activations.len(), 1);
}

#[test]
fn replay_trace_cannot_select_retirement_or_parameter_ownership() {
    fn retirement(trace: ReplayTrace) -> InputRetirementAction {
        let mut state = CommandState::default();
        let identity = state.push_token_level(
            transient_payload(&[]),
            TokenBehavior::Ordinary,
            RetirementBehavior::Pop,
            trace,
        );
        state
            .retire_exhausted_input(identity)
            .expect("trace-independent retirement")
            .action
    }

    assert_eq!(
        retirement(ReplayTrace::Stored(StoredReplayReason::EveryPar)),
        retirement(ReplayTrace::Inserted)
    );

    fn parameter_range(trace: ReplayTrace) -> (usize, usize) {
        let mut state = CommandState::default();
        let owner = push_activation(
            &mut state,
            12,
            vec![traced('a'), traced('b')].into(),
            [
                MacroArgumentRange::new(0, 2),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
            ],
        );
        state.push_token_level(
            transient_payload(&[]),
            TokenBehavior::MacroBody(owner),
            RetirementBehavior::Pop,
            ReplayTrace::MacroReplacement,
        );
        let nested = state.push_token_level(
            transient_payload(&[traced('#')]),
            TokenBehavior::Ordinary,
            RetirementBehavior::Pop,
            trace,
        );
        state
            .replay_out_parameter(nested, 1)
            .expect("trace-independent parameter replay");
        let crate::input::InputLevel::Tokens(cursor) =
            state.input.levels.last().expect("parameter replay")
        else {
            panic!("parameter replay was not a token level");
        };
        let TokenPayload::ArgumentRange { range, .. } = &cursor.payload else {
            panic!("parameter replay did not use a range");
        };
        (range.start(), range.end())
    }

    assert_eq!(
        parameter_range(ReplayTrace::Stored(StoredReplayReason::Mark)),
        parameter_range(ReplayTrace::Inserted)
    );
}

#[test]
fn stored_token_reference_lifetime_survives_redefinition_and_replay() {
    let mut universe = Universe::new();
    let symbol = universe.intern("stable-reference").symbol();
    universe.set_meaning(symbol, Meaning::CharGiven('A'));
    let list = universe.intern_token_list(&[Token::Cs(symbol)]);
    let mut state = CommandState::default();
    let level = state.push_token_level(
        TokenPayload::Stored {
            tokens: list,
            origins: OriginListId::EMPTY,
        },
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::Stored(StoredReplayReason::Mark),
    );

    universe.set_meaning(symbol, Meaning::CharGiven('B'));

    let InputLevel::Tokens(cursor) = state
        .input
        .levels
        .last()
        .expect("stored level remains live")
    else {
        panic!("stored replay changed level kind");
    };
    let TokenPayload::Stored { tokens, .. } = cursor.payload else {
        panic!("stored replay changed payload kind");
    };
    assert_eq!(universe.tokens(tokens), &[Token::Cs(symbol)]);
    assert_eq!(universe.meaning(symbol), Meaning::CharGiven('B'));
    assert_eq!(
        state
            .retire_exhausted_input(level)
            .expect("replay retires")
            .reason,
        InputRetirementReason::TokenList(StoredReplayReason::Mark)
    );
}

#[test]
fn source_level_begin_end_restore_line_and_terminal_state() {
    let mut state = CommandState::default();
    let outer = state
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            b"outer\n".to_vec(),
        ))
        .expect("outer source registers");
    state
        .open_registered_source(outer)
        .expect("outer source opens");
    let outer_line = state.load_next_source_line(13).expect("outer line loads");

    let inner = state
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            b"inner\n".to_vec(),
        ))
        .expect("inner source registers");
    state
        .open_registered_source(inner)
        .expect("inner source opens");
    let inner_identity = match state.input.levels.last().expect("inner level") {
        InputLevel::Source(source) => source.identity,
        InputLevel::Tokens(_) => panic!("opened source is not a source level"),
    };
    assert_eq!(state.input.levels.len(), 2);
    let retired = state
        .retire_exhausted_input(inner_identity)
        .expect("inner source retires");
    assert_eq!(retired.action, InputRetirementAction::SourcePopped);
    assert_eq!(retired.reason, InputRetirementReason::Source);

    let InputLevel::Source(source) = state.input.levels.last().expect("outer source restored")
    else {
        panic!("outer source changed level kind");
    };
    assert_eq!(
        source
            .cursor
            .line
            .as_ref()
            .expect("outer line remains live")
            .physical,
        outer_line
    );
    let outer_identity = source.identity;
    state
        .retire_exhausted_input(outer_identity)
        .expect("outer source retires");
    assert!(state.input.levels.is_empty());
    assert_eq!(
        state.retire_exhausted_input(outer_identity),
        Err(InputRetirementError::NoInput)
    );
}

#[test]
fn token_list_kind_reference_and_parameter_stack_lifecycle_matrix() {
    for (behavior, trace, expected_reason) in [
        (
            TokenBehavior::Ordinary,
            ReplayTrace::Stored(StoredReplayReason::Mark),
            InputRetirementReason::TokenList(StoredReplayReason::Mark),
        ),
        (
            TokenBehavior::Parameter,
            ReplayTrace::MacroParameter { slot: 9 },
            InputRetirementReason::Parameter,
        ),
        (
            TokenBehavior::BackedUp(BackupTreatment::Ordinary),
            ReplayTrace::BackedUp,
            InputRetirementReason::Backup,
        ),
    ] {
        let mut state = CommandState::default();
        let identity = state.push_token_level(
            TokenPayload::Stored {
                tokens: TokenListId::EMPTY,
                origins: OriginListId::EMPTY,
            },
            behavior,
            RetirementBehavior::Pop,
            trace,
        );
        let retirement = state
            .retire_exhausted_input(identity)
            .expect("matrix level retires");
        assert_eq!(retirement.action, InputRetirementAction::TokenListPopped);
        assert_eq!(retirement.reason, expected_reason);
        assert!(state.input.levels.is_empty());
    }
}

#[test]
fn backup_inserted_and_macro_levels_retire_in_canonical_order() {
    let mut state = CommandState::default();
    let activation = push_activation(
        &mut state,
        41,
        Vec::<TracedTokenWord>::new().into(),
        [None; 9],
    );
    let macro_level = state.push_token_level(
        transient_payload(&[]),
        TokenBehavior::MacroBody(activation),
        RetirementBehavior::Pop,
        ReplayTrace::MacroReplacement,
    );
    let inserted = state.push_token_level(
        transient_payload(&[]),
        TokenBehavior::Recovery,
        RetirementBehavior::Pop,
        ReplayTrace::Inserted,
    );
    let backup = state.push_token_level(
        TokenPayload::BackedUp(SharedBackedUpBuffer::default()),
        TokenBehavior::BackedUp(BackupTreatment::Ordinary),
        RetirementBehavior::Pop,
        ReplayTrace::BackedUp,
    );

    for (identity, reason) in [
        (backup, InputRetirementReason::Backup),
        (inserted, InputRetirementReason::Recovery),
        (macro_level, InputRetirementReason::Macro),
    ] {
        assert_eq!(
            state
                .retire_exhausted_input(identity)
                .expect("top level retires")
                .reason,
            reason
        );
    }
    assert!(state.input.levels.is_empty());
    assert!(state.parameters.activations.is_empty());
}

#[test]
fn input_level_retirement_covers_source_token_parameter_and_reference_actions() {
    for (behavior, retirement, trace, action, reason) in [
        (
            TokenBehavior::Ordinary,
            RetirementBehavior::StopAtEnd,
            ReplayTrace::Stored(StoredReplayReason::EveryJob),
            InputRetirementAction::TerminalStop,
            InputRetirementReason::TokenList(StoredReplayReason::EveryJob),
        ),
        (
            TokenBehavior::Parameter,
            RetirementBehavior::Pop,
            ReplayTrace::MacroParameter { slot: 1 },
            InputRetirementAction::TokenListPopped,
            InputRetirementReason::Parameter,
        ),
        (
            TokenBehavior::Recovery,
            RetirementBehavior::CloseScantokens,
            ReplayTrace::Transient(TransientReplayReason::Scantokens),
            InputRetirementAction::ScantokensClosed,
            InputRetirementReason::Recovery,
        ),
    ] {
        let mut state = CommandState::default();
        let identity = state.push_token_level(transient_payload(&[]), behavior, retirement, trace);
        let retired = state
            .retire_exhausted_input(identity)
            .expect("level retires");
        assert_eq!(retired.action, action);
        assert_eq!(retired.reason, reason);
    }
}
