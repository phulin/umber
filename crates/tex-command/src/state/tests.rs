use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use super::{
    CommandRuntime, CommandState, MeaningCacheEntry, NormalizedLineCacheEntry, TransientState,
};
use crate::conditionals::ConditionStack;
use crate::input::{FileFramingEvent, InputState};
use crate::macro_call::ParameterState;
use crate::processor::{AlignmentDeliveryState, ExpansionState, ScannerState};
use crate::{
    AlignmentCellTemplates, AlignmentIdentity, AlignmentLifecycleError, AlignmentRequest,
    AlignmentRequestResult, RegisteredSourceKind, SourceNameClass, SourceRegistration,
};

fn templates() -> AlignmentCellTemplates {
    AlignmentCellTemplates {
        u_template: None,
        v_template: tex_state::input::TracedTokenList::synthetic(
            tex_state::ids::TokenListId::EMPTY,
        ),
    }
}

fn semantic_hash(state: &CommandState) -> u64 {
    let mut hasher = DefaultHasher::new();
    state.hash(&mut hasher);
    hasher.finish()
}

#[test]
fn rebuilding_runtime_does_not_change_semantic_state() {
    let mut state = CommandState::default();
    state.input.next_level_identity = 3;
    state.parameters.activations.reserve(5);
    state.conditions.next_identity = 7;
    state.alignment.align_state = 9;
    state.expansion.cumulative_expansions = 11;
    state.transient.next_builder_identity = 13;
    let original = state.clone();
    let original_hash = semantic_hash(&state);
    let original_summary = state
        .publish_summary()
        .expect("the populated state is quiescent");

    let mut runtime = CommandRuntime::default();
    runtime.meaning_cache.entries.push(MeaningCacheEntry {
        identity: 7,
        generation: 11,
    });
    runtime
        .normalized_lines
        .entries
        .push(NormalizedLineCacheEntry {
            content_identity: 13,
            normalized: b"normalized".to_vec(),
        });
    runtime.transient_pool.buffers.push(Vec::new());
    runtime.profiling.raw_deliveries = 17;
    runtime.profiling.cache_hits = 19;

    runtime = CommandRuntime::default();

    assert_eq!(state, original);
    assert_eq!(semantic_hash(&state), original_hash);
    assert_eq!(
        state
            .publish_summary()
            .expect("runtime replacement cannot change quiescence"),
        original_summary
    );
    assert!(runtime.meaning_cache.entries.is_empty());
    assert!(runtime.normalized_lines.entries.is_empty());
    assert!(runtime.transient_pool.buffers.is_empty());
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
        registered_sources,
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
    } = alignment;
    assert!(completed_preamble.is_none());
    assert!(pending_fin_col_delimiter.is_none());
    assert!(extra_tab_recovery.is_none());
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
        registered_sources,
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
        let alignment = AlignmentIdentity::new(41);
        state.begin_alignment(alignment);
        state.alignment.pending_fin_col_delimiter = Some((alignment, delimiter));

        assert_eq!(
            state
                .apply_alignment_request(AlignmentRequest::RecoverExtraTab(alignment))
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
    assert!(state.input.registered_sources.is_empty());
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
        .begin_alignment_cell(outer, outer_templates)
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
    let outer = AlignmentIdentity::new(47);
    let inner = AlignmentIdentity::new(53);

    assert_eq!(
        state.apply_alignment_request(AlignmentRequest::Begin(outer)),
        Ok(AlignmentRequestResult::Applied)
    );
    assert_eq!(
        state.apply_alignment_request(AlignmentRequest::BeginCell {
            alignment: outer,
            templates: templates(),
        }),
        Ok(AlignmentRequestResult::Applied)
    );
    assert_eq!(
        state.apply_alignment_request(AlignmentRequest::Suspend(outer)),
        Ok(AlignmentRequestResult::Applied)
    );
    assert_eq!(
        state.apply_alignment_request(AlignmentRequest::Begin(inner)),
        Ok(AlignmentRequestResult::Applied)
    );
    assert_eq!(
        state.apply_alignment_request(AlignmentRequest::Finish(inner)),
        Ok(AlignmentRequestResult::Applied)
    );
    assert_eq!(
        state.apply_alignment_request(AlignmentRequest::Resume(outer)),
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
        crate::input::InputLevel::Source(level) => level.identity,
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
