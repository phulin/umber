use std::sync::Arc;

use tex_state::Universe;
use tex_state::ids::TokenListId;
use tex_state::meaning::Meaning;
use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};

use crate::macro_call::{MacroActivationId, MacroArgumentRange};
use crate::{CommandState, RegisteredSourceKind, SourceNameClass, SourceRegistration};

use super::{
    InputRetirementAction, InputRetirementError, InputRetirementReason, OutParameterReplay,
    ParameterReplayError, input_level_identity,
};
use crate::input::levels::{
    BackupTreatment, InputLevel, ReplayTrace, RetirementBehavior, StoredReplayReason,
    TokenBehavior, TokenPayload, TransientReplayReason,
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
    TokenPayload::transient(tokens.iter().copied())
}

fn push_activation(
    state: &mut CommandState,
    identity: u64,
    tokens: Arc<[TracedTokenWord]>,
    ranges: [Option<MacroArgumentRange>; 9],
) -> MacroActivationId {
    let identity = MacroActivationId(identity);
    let arguments = state.parameters.store_arguments(
        tex_state::token::RootedTracedTokenBuffer::new(
            tokens
                .iter()
                .copied()
                .map(tex_state::token::RootedTracedTokenWord::unowned),
        ),
        ranges,
    );
    state.parameters.restore_activation(
        identity,
        tex_state::interner::Symbol::testing_new(1),
        tex_state::macro_store::MacroDefinitionRef::testing_new(
            u32::try_from(identity.0 + 100).expect("test definition identity fits"),
        )
        .id(),
        arguments,
        tex_state::token::OriginId::UNKNOWN,
    );
    identity
}

#[test]
fn transient_dynamic_words_count_owned_buffers_once() {
    let mut state = CommandState::default();
    let arguments = Arc::from([traced('a'), traced('b'), traced('c')]);
    push_activation(
        &mut state,
        1,
        Arc::clone(&arguments),
        [
            MacroArgumentRange::new(0, 3),
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
    let activation_arguments = state.parameters.activations[0].arguments;
    state.push_token_level(
        TokenPayload::ArgumentRange {
            arguments: activation_arguments,
            range: MacroArgumentRange::new(0, 3).expect("valid range"),
        },
        TokenBehavior::Parameter,
        RetirementBehavior::Pop,
        ReplayTrace::MacroParameter { slot: 1 },
    );
    state.push_token_level(
        transient_payload(&[traced('x'), traced('y')]),
        TokenBehavior::Recovery,
        RetirementBehavior::Pop,
        ReplayTrace::Inserted,
    );
    state.push_token_level(
        TokenPayload::stored(
            &[Token::Char {
                ch: 's',
                cat: Catcode::Other,
            }; 4],
            std::iter::empty(),
        ),
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::Stored(StoredReplayReason::EveryJob),
    );

    // TeX82 §§357/390: the argument-range cursor shares its activation's
    // three token nodes, the recovery list owns two more, and replaying the
    // four-word stored list only adds a reference. Neither shared nor stored
    // tokens are duplicated merely because every host payload is packed.
    assert_eq!(state.transient_dynamic_words(), 5);
}

#[test]
fn each_popped_level_retires_exactly_once_with_its_trace() {
    let mut state = CommandState::default();
    let universe = Universe::new();
    let identity = state.push_token_level(
        TokenPayload::stored(
            universe.tokens(TokenListId::EMPTY).tokens(),
            std::iter::empty(),
        ),
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
        RetirementBehavior::Pop,
        ReplayTrace::Stored(StoredReplayReason::EveryEof),
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
        InputRetirementAction::TokenListPopped
    );
}

#[test]
fn file_source_retirement_clears_process_global_force_eof() {
    let mut state = CommandState::default();
    let source = state
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(&b"x"[..]),
        ))
        .expect("source registers");
    state.open_registered_source(source).expect("source opens");
    let identity = input_level_identity(state.input.levels.last().expect("source level is live"));
    state.input.force_eof = true;

    state
        .retire_exhausted_input(identity)
        .expect("forced source retires");

    assert!(
        !state.input.force_eof,
        "TeX82 §362 clears true immediately before end_file_reading"
    );
}

#[test]
fn pseudo_source_retirement_preserves_process_global_force_eof() {
    for name_class in [
        SourceNameClass::Terminal,
        SourceNameClass::ReadStream(0),
        SourceNameClass::ReadStream(16),
    ] {
        let mut state = CommandState::default();
        let source = state
            .register_source(SourceRegistration::new(
                RegisteredSourceKind::Generated,
                Arc::<[u8]>::from(&b"x"[..]),
            ))
            .expect("source registers");
        state
            .open_registered_source_as(source, name_class)
            .expect("pseudo-source opens");
        let identity =
            input_level_identity(state.input.levels.last().expect("source level is live"));
        state.input.force_eof = true;

        state
            .retire_exhausted_input(identity)
            .expect("pseudo-source retires");

        assert!(
            state.input.force_eof,
            "TeX82 §§360–362 leave force_eof untouched for {name_class:?}"
        );
    }
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
fn transient_payload_is_packed_at_construction() {
    let mut state = CommandState::default();
    let identity = state.push_token_level(
        TokenPayload::transient([traced('x')]),
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::Inserted,
    );
    assert!(matches!(
        state.input.levels.last(),
        Some(InputLevel::Tokens(cursor)) if matches!(cursor.payload, TokenPayload::Packed(_))
    ));

    state
        .retire_exhausted_input(identity)
        .expect("transient insertion retires");
}

#[test]
fn snapshot_ownership_restores_packed_payload_after_stack_retirement() {
    let mut state = CommandState::default();
    let identity = state.push_token_level(
        TokenPayload::transient([traced('x')]),
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::Inserted,
    );
    let snapshot = state.snapshot();

    state
        .retire_exhausted_input(identity)
        .expect("live cursor retires");
    state.rollback(snapshot).expect("snapshot restores");
    assert!(matches!(
        state.input.levels.last(),
        Some(InputLevel::Tokens(cursor))
            if matches!(&cursor.payload, TokenPayload::Packed(chunk) if chunk.word(0) == Some(traced('x')))
    ));
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
    assert_eq!(cursor.identity(), parameter);
    assert_eq!(cursor.behavior, TokenBehavior::Parameter);
    assert_eq!(cursor.trace, ReplayTrace::MacroParameter { slot: 2 });
    let TokenPayload::ArgumentRange { arguments, range } = &cursor.payload else {
        panic!("parameter replay did not share its activation buffer");
    };
    assert_eq!(state.parameters.argument_words(*arguments).len(), 2);
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
    let list = universe.intern_token_list_ref(&[Token::Cs(symbol)]);
    let mut state = CommandState::default();
    let level = state.push_token_level(
        TokenPayload::stored(universe.tokens(list.id()).tokens(), std::iter::empty()),
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
    let TokenPayload::Packed(chunk) = &cursor.payload else {
        panic!("stored replay was not admitted to a packed chunk");
    };
    assert_eq!(
        chunk.word(0).map(|word| word.semantic_token()),
        Some(Token::Cs(symbol))
    );
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
fn snapshot_restores_strong_stored_root_after_live_cursor_retires() {
    let mut universe = Universe::new();
    let root = universe.intern_token_list_ref(&[Token::Char {
        ch: 'R',
        cat: Catcode::Other,
    }]);
    let mut state = CommandState::default();
    let level = state.push_token_level(
        TokenPayload::stored(universe.tokens(root.id()).tokens(), std::iter::empty()),
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::Stored(StoredReplayReason::Mark),
    );
    let snapshot = state.snapshot();

    state
        .retire_exhausted_input(level)
        .expect("live stored cursor retires");
    drop(universe);
    state.rollback(snapshot).expect("snapshot restores");

    let InputLevel::Tokens(cursor) = state.input.levels.last().expect("restored cursor") else {
        panic!("snapshot restored the wrong level kind");
    };
    let TokenPayload::Packed(chunk) = &cursor.payload else {
        panic!("snapshot restored the wrong payload kind");
    };
    assert_eq!(
        chunk.word(0).map(|word| word.semantic_token()),
        Some(Token::Char {
            ch: 'R',
            cat: Catcode::Other,
        })
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
        InputLevel::Source(source) => source.identity(),
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
    let outer_identity = source.identity();
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
fn source_retirement_reports_tex_web_303_name_classification() {
    // tex.web §303 partitions a source level's `name` into the terminal
    // (`name=0`), input stream `name-1` (`1<=name<=17`), and a text file
    // (`name>17`). §328's `begin_file_reading` opens every level at `name=0`,
    // §483's `read_toks` sets `name:=m+1`, and §537's `start_input` installs a
    // file's string number; §329's `end_file_reading` acts on the last of
    // those alone. Retirement therefore has to report which one ended, and a
    // §307 `token_type` -- what `InputRetirementReason` models -- cannot say.
    for class in [
        SourceNameClass::Terminal,
        SourceNameClass::ReadStream(0),
        SourceNameClass::ReadStream(16),
        SourceNameClass::File,
    ] {
        let mut state = CommandState::default();
        let source = state
            .register_source(SourceRegistration::new(
                RegisteredSourceKind::Generated,
                b"x\n".to_vec(),
            ))
            .expect("source registers");
        state
            .open_registered_source_as(source, class)
            .expect("source opens");
        let InputLevel::Source(level) = state.input.levels.last().expect("source level") else {
            panic!("opened source is not a source level");
        };
        assert_eq!(level.name_class, class);
        let identity = level.identity();
        let retired = state
            .retire_exhausted_input(identity)
            .expect("source retires");
        assert_eq!(retired.action, InputRetirementAction::SourcePopped);
        assert_eq!(retired.reason, InputRetirementReason::Source);
        assert_eq!(retired.name_class, Some(class));
    }
}

#[test]
fn ordinary_source_open_classifies_as_tex_web_537_start_input_file() {
    // Every `\input`, and the job's own root file, reaches TeX through §537's
    // `start_input`, so an unqualified open is a file and never the terminal
    // §328 would have left it as.
    let mut state = CommandState::default();
    let source = state
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Vec::new(),
        ))
        .expect("source registers");
    state.open_registered_source(source).expect("source opens");
    let InputLevel::Source(level) = state.input.levels.last().expect("source level") else {
        panic!("opened source is not a source level");
    };
    assert_eq!(level.name_class, SourceNameClass::File);
}

#[test]
fn token_list_retirement_reports_no_name_classification() {
    // §307 reuses `name` on a token-list level as the eqtb address of the
    // macro being expanded, so §303's classification does not apply at all.
    let mut state = CommandState::default();
    let identity = state.push_token_level(
        transient_payload(&[traced('x')]),
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::Inserted,
    );
    let retired = state
        .retire_exhausted_input(identity)
        .expect("token list retires");
    assert_eq!(retired.name_class, None);
}

#[test]
fn token_list_kind_reference_and_parameter_stack_lifecycle_matrix() {
    let universe = Universe::new();
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
            TokenPayload::stored(
                universe.tokens(TokenListId::EMPTY).tokens(),
                std::iter::empty(),
            ),
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
        TokenPayload::backed_up([]),
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
            RetirementBehavior::Pop,
            ReplayTrace::Stored(StoredReplayReason::EveryEof),
            InputRetirementAction::TokenListPopped,
            InputRetirementReason::TokenList(StoredReplayReason::EveryEof),
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
