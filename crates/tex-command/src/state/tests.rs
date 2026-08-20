use super::{
    CommandState, MAX_RETAINED_TRACED_TOKEN_CAPACITY, TRACED_TOKEN_POOL_SLOTS,
    TracedTokenBufferPool, TransientState, traced_token_scratch_from,
};
use crate::conditionals::ConditionStack;
use crate::input::{FileFramingEvent, InputState};
use crate::macro_call::ParameterState;
use crate::processor::{AlignmentDeliveryState, ExpansionState, ScannerState};
use crate::{
    AlignmentCellTemplates, AlignmentIdentity, AlignmentLifecycleError, AlignmentRequest,
    AlignmentRequestResult, RegisteredSourceKind, SourceFramingPolicy, SourceNameClass,
    SourceRegistration,
};
use std::sync::Arc;
use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};

fn traced_a() -> TracedTokenWord {
    TracedTokenWord::pack(
        Token::Char {
            ch: 'a',
            cat: Catcode::Letter,
        },
        OriginId::UNKNOWN,
    )
}

#[test]
fn terminal_buffer_usage_is_runtime_only_and_monotonic() {
    let mut command = CommandState::default();
    command.set_terminal_context_line("trip  ");
    let semantic = command.clone();

    assert_eq!(command.stack_usage().buffer_stack, 7);
    command.usage.record_buffer_usage(19);
    assert_eq!(command, semantic);
    command.set_terminal_context_line("x");
    assert_eq!(command.stack_usage().buffer_stack, 19);
}

fn retained_capacities(pool: &TracedTokenBufferPool) -> Vec<usize> {
    pool.buffers
        .lock()
        .expect("traced-token pool lock is not poisoned")
        .iter()
        .filter_map(|buffer| buffer.as_ref().map(|buffer| buffer.capacity()))
        .collect()
}

fn fail_with_checked_out_scratch(pool: Arc<TracedTokenBufferPool>) -> Result<(), ()> {
    let mut scratch = traced_token_scratch_from(pool);
    scratch.push_unowned(traced_a());
    Err(())
}

#[test]
fn traced_token_scratch_returns_on_success_and_error() {
    let pool = Arc::new(TracedTokenBufferPool::default());
    {
        let mut scratch = traced_token_scratch_from(Arc::clone(&pool));
        scratch.extend_unowned([traced_a(); 17]);
    }
    assert_eq!(retained_capacities(&pool).len(), 1);

    let result = fail_with_checked_out_scratch(Arc::clone(&pool));
    assert_eq!(result, Err(()));
    assert_eq!(retained_capacities(&pool).len(), 1);
}

#[test]
fn traced_token_scratch_pool_bounds_slots_and_capacity() {
    let pool = Arc::new(TracedTokenBufferPool::default());
    let mut checkouts = (0..=TRACED_TOKEN_POOL_SLOTS)
        .map(|_| traced_token_scratch_from(Arc::clone(&pool)))
        .collect::<Vec<_>>();
    for scratch in &mut checkouts {
        scratch.push_unowned(traced_a());
    }
    drop(checkouts);
    assert_eq!(retained_capacities(&pool).len(), TRACED_TOKEN_POOL_SLOTS);

    let pool = Arc::new(TracedTokenBufferPool::default());
    {
        let mut oversized = traced_token_scratch_from(Arc::clone(&pool));
        oversized.reserve_exact(MAX_RETAINED_TRACED_TOKEN_CAPACITY + 1);
    }
    assert!(retained_capacities(&pool).is_empty());
}

fn templates() -> AlignmentCellTemplates {
    let universe = tex_state::Universe::new();
    AlignmentCellTemplates {
        u_template: None,
        v_template: tex_state::input::TracedTokenList::synthetic(
            universe.token_list_ref(tex_state::ids::TokenListId::EMPTY),
        ),
    }
}

#[test]
fn semantic_ownership_domains_are_exhaustively_classified() {
    let CommandState {
        input,
        parameters,
        scanner,
        conditions,
        alignment,
        expansion,
        transient,
        semantic_diagnostics,
        ..
    } = CommandState::default();

    let InputState {
        levels,
        terminal_context_line,
        pending_sources,
        next_level_identity,
        next_source_identity,
        force_eof,
    } = input;
    let ParameterState { activations, .. } = parameters;
    let ScannerState { .. } = scanner;
    let ConditionStack {
        frames,
        next_identity,
    } = conditions;
    let AlignmentDeliveryState {
        align_state,
        align_stack,
        active_alignment,
        suspended,
        active_cell,
        completed_preamble,
        pending_fin_col_delimiter,
        extra_tab_recovery,
        pending_outer_recovery_cr,
    } = alignment;
    assert!(completed_preamble.is_none());
    assert!(pending_fin_col_delimiter.is_none());
    assert!(extra_tab_recovery.is_none());
    assert!(pending_outer_recovery_cr.is_none());
    let ExpansionState {
        cumulative_expansions,
        next_resource_resolution,
        pending_diagnostics,
        observed_dependencies,
        semantic_barriers,
        profile,
    } = expansion;
    let dialect = profile.dialect();
    let characters = profile.character_mode();
    let TransientState {
        builders,
        rollback_roots,
        next_builder_identity,
        active_expansion_depth,
    } = transient;

    drop((
        levels,
        terminal_context_line,
        pending_sources,
        next_level_identity,
        next_source_identity,
        force_eof,
        activations,
        frames,
        next_identity,
        align_state,
        align_stack,
        active_alignment,
        suspended,
        active_cell,
        cumulative_expansions,
        next_resource_resolution,
        pending_diagnostics,
        observed_dependencies,
        semantic_barriers,
        dialect,
        characters,
        builders,
        rollback_roots,
        next_builder_identity,
        active_expansion_depth,
        semantic_diagnostics,
    ));
}

#[test]
fn fin_col_extra_tab_recovery_is_command_owned_and_accepts_span_too() {
    for delimiter in [
        crate::AlignmentCellDelimiter::Tab,
        crate::AlignmentCellDelimiter::Span,
    ] {
        let mut state = CommandState::default();
        let mut universe = tex_state::Universe::new();
        let alignment = AlignmentIdentity::new(41);
        state.begin_alignment(alignment);
        state.alignment.pending_fin_col_delimiter = Some((alignment, delimiter));

        assert_eq!(
            state
                .apply_alignment_request(
                    &universe.command_context(),
                    AlignmentRequest::RecoverExtraTab(alignment),
                )
                .expect("TeX82 fin_col converts an exhausted tab/span to cr"),
            AlignmentRequestResult::ExtraTabRecovered,
        );
        assert_eq!(
            state.alignment.pending_fin_col_delimiter,
            Some((alignment, crate::AlignmentCellDelimiter::Row)),
        );
        assert_eq!(state.alignment.extra_tab_recovery, Some(alignment));
    }
}

#[test]
fn default_state_is_quiescent() {
    let state = CommandState::default();

    assert!(state.input.levels.is_empty());
    assert!(state.input.pending_sources.is_empty());
    assert!(state.parameters.activations.is_empty());
    assert!(state.conditions.frames.is_empty());
    assert!(state.alignment.suspended.is_empty());
    assert!(state.alignment.active_cell.is_none());
    assert!(state.expansion.pending_diagnostics.is_empty());
    assert!(state.semantic_diagnostics.is_empty());
    assert!(state.transient.builders.is_empty());
    assert_eq!(state.transient.active_expansion_depth, 0);
}

#[test]
fn nested_alignment_suspension_restores_the_outer_cell_identity_and_templates() {
    let mut state = CommandState::default();
    let outer = AlignmentIdentity::new(41);
    let inner = AlignmentIdentity::new(43);
    let outer_templates = templates();

    state.begin_alignment(outer);
    state
        .begin_alignment_cell(outer, outer_templates.clone())
        .expect("outer cell begins");
    state.suspend_alignment(outer).expect("outer cell suspends");
    assert_eq!(
        state.resume_alignment(inner),
        Err(AlignmentLifecycleError::WrongAlignment)
    );

    state.begin_alignment(inner);
    state
        .begin_alignment_cell(inner, templates())
        .expect("inner cell begins");
    state
        .finish_alignment(inner)
        .expect("inner alignment finishes");
    state
        .resume_alignment(outer)
        .expect("outer alignment resumes");
    assert_eq!(
        state
            .alignment
            .active_cell
            .as_ref()
            .expect("outer cell restores")
            .templates,
        outer_templates
    );
}

#[test]
fn typed_requests_preserve_nested_alignment_delivery_without_token_classification() {
    let mut state = CommandState::default();
    let mut universe = tex_state::Universe::new();
    let outer = AlignmentIdentity::new(47);
    let inner = AlignmentIdentity::new(53);

    assert_eq!(
        state.apply_alignment_request(&universe.command_context(), AlignmentRequest::Begin(outer)),
        Ok(AlignmentRequestResult::Applied)
    );
    assert_eq!(
        state.apply_alignment_request(
            &universe.command_context(),
            AlignmentRequest::BeginCell {
                alignment: outer,
                templates: templates(),
            },
        ),
        Ok(AlignmentRequestResult::Applied)
    );
    assert_eq!(
        state.apply_alignment_request(
            &universe.command_context(),
            AlignmentRequest::Suspend(outer),
        ),
        Ok(AlignmentRequestResult::Applied)
    );
    assert_eq!(
        state.apply_alignment_request(&universe.command_context(), AlignmentRequest::Begin(inner)),
        Ok(AlignmentRequestResult::Applied)
    );
    assert_eq!(
        state
            .apply_alignment_request(&universe.command_context(), AlignmentRequest::Finish(inner),),
        Ok(AlignmentRequestResult::Applied)
    );
    assert_eq!(
        state
            .apply_alignment_request(&universe.command_context(), AlignmentRequest::Resume(outer),),
        Ok(AlignmentRequestResult::Applied)
    );
    assert_eq!(
        state
            .alignment
            .active_cell
            .as_ref()
            .expect("outer cell resumes")
            .templates,
        templates()
    );
}

fn register_named(state: &mut CommandState, name: &str, bytes: &[u8]) -> tex_state::SourceId {
    state
        .register_source(
            SourceRegistration::new(RegisteredSourceKind::Generated, bytes.to_vec())
                .with_name(name),
        )
        .expect("named source registers")
}

fn source_level_identity(state: &CommandState) -> crate::input::InputLevelId {
    match state.input.levels.last().expect("source level opened") {
        crate::input::InputLevel::Source(level) => level.identity(),
        crate::input::InputLevel::Tokens(_) => panic!("opened source is not a source level"),
    }
}

#[test]
fn opening_a_named_file_source_queues_exactly_one_open_event() {
    // §537's `start_input` prints `(` and the opened file's name; a `File`
    // open is the only source open this queue ever reports for.
    let mut state = CommandState::default();
    let source = register_named(&mut state, "show-box.tex", b"\\showbox0\n");

    state
        .open_registered_source(source)
        .expect("named source opens as a text file");

    assert_eq!(
        state.take_file_framing_events(),
        vec![FileFramingEvent::Open {
            name: "show-box.tex".into(),
        }]
    );
}

#[test]
fn current_file_source_identity_matches_the_line_owner() {
    let mut state = CommandState::default();
    let source = register_named(&mut state, "show-box.tex", b"x\n");
    state
        .open_registered_source(source)
        .expect("named source opens as a text file");

    assert_eq!(state.current_file_source_id(), Some(source));
}

#[test]
fn opening_consumes_pending_backing_and_reopening_the_id_fails() {
    let mut state = CommandState::default();
    let source = register_named(&mut state, "one-shot.tex", b"x\n");

    assert_eq!(state.input.pending_sources.len(), 1);
    state
        .open_registered_source(source)
        .expect("pending source opens once");

    assert!(state.input.pending_sources.is_empty());
    assert_eq!(
        state
            .open_registered_source(source)
            .expect_err("consumed source cannot reopen")
            .source(),
        source
    );
}

#[test]
fn retiring_an_opened_source_releases_its_backing() {
    let mut state = CommandState::default();
    let bytes: std::sync::Arc<[u8]> = std::sync::Arc::from(&b"x\n"[..]);
    let weak = std::sync::Arc::downgrade(&bytes);
    let source = state
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            std::sync::Arc::clone(&bytes),
        ))
        .expect("source registers");
    drop(bytes);
    state
        .open_registered_source(source)
        .expect("source opens once");
    let identity = source_level_identity(&state);

    state
        .retire_exhausted_input(identity)
        .expect("source level retires");

    assert!(state.input.pending_sources.is_empty());
    assert!(weak.upgrade().is_none());
}

#[test]
fn exhausting_a_named_file_source_queues_exactly_one_close_event() {
    // §362 prints `)` once a text file's last line is consumed.
    let mut state = CommandState::default();
    let source = register_named(&mut state, "show-box.tex", b"");
    state
        .open_registered_source(source)
        .expect("named source opens as a text file");
    let identity = source_level_identity(&state);
    let _ = state.take_file_framing_events();

    state
        .retire_exhausted_input(identity)
        .expect("the exact opened level retires");

    assert_eq!(
        state.take_file_framing_events(),
        vec![FileFramingEvent::Close]
    );
}

#[test]
fn externally_framed_named_source_retains_file_identity_without_events() {
    let mut state = CommandState::default();
    let source = state
        .register_source(
            SourceRegistration::new(RegisteredSourceKind::Generated, b"x\n".to_vec())
                .with_name("/job/main.tex")
                .with_framing(SourceFramingPolicy::ExternallyOwned),
        )
        .expect("externally framed source registers");

    state
        .open_registered_source(source)
        .expect("named source opens as a file");
    assert!(state.take_file_framing_events().is_empty());
    let identity = source_level_identity(&state);
    let crate::input::InputLevel::Source(level) = state.input.levels.last().expect("root level")
    else {
        panic!("root must remain a source level");
    };
    assert_eq!(level.name_class, SourceNameClass::File);

    state
        .retire_exhausted_input(identity)
        .expect("externally framed source retires");
    assert!(state.take_file_framing_events().is_empty());
}

#[test]
fn read_stream_and_terminal_source_levels_queue_no_framing_events() {
    // §331's terminal and §483's `\read` streams are never bracketed by
    // tex.web, even when their registration happens to carry a name.
    for class in [SourceNameClass::Terminal, SourceNameClass::ReadStream(0)] {
        let mut state = CommandState::default();
        let source = register_named(&mut state, "would-not-print.tex", b"x\n");
        state
            .open_registered_source_as(source, class)
            .expect("source opens under the requested classification");
        let identity = source_level_identity(&state);

        assert!(state.take_file_framing_events().is_empty());

        state
            .retire_exhausted_input(identity)
            .expect("the exact opened level retires");

        assert!(state.take_file_framing_events().is_empty());
    }
}

#[test]
fn scantokens_framing_uses_a_space_name_only_when_traced() {
    // e-TeX 2.6 manual §3.2: numeric pseudo-file name 19 records opening and
    // closing like a file whose displayed name is one space; name 18 is
    // silent when `\tracingscantokens<=0`.
    for (numeric_name, expected) in [
        (18, Vec::new()),
        (
            19,
            vec![
                FileFramingEvent::Open { name: " ".into() },
                FileFramingEvent::Close,
            ],
        ),
    ] {
        let mut state = CommandState::default();
        let identity = state
            .open_scantokens(
                SourceRegistration::new(RegisteredSourceKind::Generated, b"x\n".to_vec()),
                None,
                numeric_name,
            )
            .expect("scantokens pseudo-file opens");
        state
            .retire_exhausted_input(identity)
            .expect("scantokens pseudo-file retires");
        assert_eq!(state.take_file_framing_events(), expected);
    }
}

#[test]
fn unnamed_file_class_source_queues_no_close_without_a_matching_open() {
    // `push_source_level` only queues `Open` when the registration carries a
    // §537 name (`SourceRegistration::with_name`); a `File`-classed source
    // with none -- every registration built before this queue existed, and
    // any future one that forgets to name it -- must not queue an orphan
    // `Close` either, or the engine would print an unbalanced `)` for a
    // paren it never opened.
    let mut state = CommandState::default();
    let source = state
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            b"x\n".to_vec(),
        ))
        .expect("unnamed source registers");
    state
        .open_registered_source(source)
        .expect("unnamed source opens as a text file");
    let identity = source_level_identity(&state);

    assert!(state.take_file_framing_events().is_empty());

    state
        .retire_exhausted_input(identity)
        .expect("the exact opened level retires");

    assert!(state.take_file_framing_events().is_empty());
}

#[test]
fn draining_file_framing_events_twice_yields_them_only_once() {
    let mut state = CommandState::default();
    let source = register_named(&mut state, "once.tex", b"x\n");
    state
        .open_registered_source(source)
        .expect("named source opens as a text file");

    assert_eq!(state.take_file_framing_events().len(), 1);
    assert!(state.take_file_framing_events().is_empty());
}

#[test]
fn nested_file_opens_and_closes_queue_in_exact_occurrence_order() {
    // An inner `\input` opens and closes entirely inside the outer file's
    // lifetime, so the queue must read open-open-close-close: the order the
    // transitions happened in, not the order the levels were popped from.
    let mut state = CommandState::default();
    let outer = register_named(&mut state, "outer.tex", b"");
    state
        .open_registered_source(outer)
        .expect("outer source opens");
    let inner = register_named(&mut state, "inner.tex", b"");
    state
        .open_registered_source(inner)
        .expect("inner source opens");
    let inner_identity = source_level_identity(&state);
    state
        .retire_exhausted_input(inner_identity)
        .expect("inner source retires");
    let outer_identity = source_level_identity(&state);
    state
        .retire_exhausted_input(outer_identity)
        .expect("outer source retires");

    assert_eq!(
        state.take_file_framing_events(),
        vec![
            FileFramingEvent::Open {
                name: "outer.tex".into(),
            },
            FileFramingEvent::Open {
                name: "inner.tex".into(),
            },
            FileFramingEvent::Close,
            FileFramingEvent::Close,
        ]
    );
}
