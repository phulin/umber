use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};

use super::{CommandSemanticDiagnostic, CommandState};
use crate::processor::AlignmentIdentity;
use crate::{AttemptError, CommandObservation, CommandObserver, InputReason, InputTransition};

fn word(ch: char) -> TracedTokenWord {
    TracedTokenWord::pack(
        Token::Char {
            ch,
            cat: Catcode::Other,
        },
        OriginId::UNKNOWN,
    )
}

#[test]
#[cfg(feature = "profiling")]
fn active_source_lookup_is_one_top_read_at_one_and_4096_replay_levels() {
    fn run(depth: usize) -> ((u64, u64, u64, u64), u64, u64) {
        crate::test_harness::with_universe(|universe| {
            let mut state = CommandState::default();
            let source = state
                .register_source(crate::SourceRegistration::new(
                    crate::RegisteredSourceKind::Generated,
                    std::sync::Arc::<[u8]>::from(&b"x"[..]),
                ))
                .expect("source context fixture source");
            state
                .open_registered_source(source)
                .expect("source context fixture opens");
            let tokens = universe
                .command_context()
                .expect("source context fixture token context")
                .allocate_token_list(&[word('x').token_word()])
                .expect("source context fixture token list");
            for _ in 0..depth {
                state.push_everypar(
                    &universe
                        .command_context()
                        .expect("source context fixture replay context"),
                    tokens.clone(),
                );
            }

            assert_eq!(state.current_file_source_id(), Some(source));
            state.profile_reset_input_source_context_counters();
            let owner = tex_state::measurement::HotCoreAllocationOwner::DeliveryAndScan;
            let before = tex_state::measurement::hot_core_thread_allocation_measurement(owner);
            {
                let _scope = tex_state::measurement::hot_core_allocation_scope(owner);
                for _ in 0..4_096 {
                    std::hint::black_box(state.current_file_source_id());
                }
            }
            let after = tex_state::measurement::hot_core_thread_allocation_measurement(owner);
            (
                state.profile_input_source_context_counters(),
                after.calls - before.calls,
                after.requested_bytes - before.requested_bytes,
            )
        })
    }

    let shallow = run(1);
    let deep = run(4_096);
    assert_eq!(shallow, ((4_096, 0, 0, 0), 0, 0));
    assert_eq!(deep, shallow);
}

#[test]
fn semantic_diagnostic_transfer_moves_the_ordered_allocation_without_allocating() {
    let mut state = CommandState::<()>::default();
    state
        .semantic_diagnostics
        .push(CommandSemanticDiagnostic::Trace {
            text: "first".to_owned(),
            force_newline: false,
        });
    state
        .semantic_diagnostics
        .push(CommandSemanticDiagnostic::MissingNumber {
            context: "second".to_owned(),
            site: None,
        });
    state
        .semantic_diagnostics
        .push(CommandSemanticDiagnostic::PdfExpansionMessage {
            text: "third".to_owned(),
        });
    let allocation = state.semantic_diagnostics.as_ptr();
    let capacity = state.semantic_diagnostics.capacity();

    #[cfg(feature = "profiling")]
    let owner = tex_state::measurement::HotCoreAllocationOwner::DeliveryAndScan;
    #[cfg(feature = "profiling")]
    let before = tex_state::measurement::hot_core_thread_allocation_measurement(owner);
    let diagnostics;
    {
        #[cfg(feature = "profiling")]
        let _scope = tex_state::measurement::hot_core_allocation_scope(owner);
        diagnostics = state.take_semantic_diagnostics();
    }
    #[cfg(feature = "profiling")]
    let after = tex_state::measurement::hot_core_thread_allocation_measurement(owner);

    #[cfg(feature = "profiling")]
    assert_eq!(after.calls - before.calls, 0);
    #[cfg(feature = "profiling")]
    assert_eq!(after.requested_bytes - before.requested_bytes, 0);
    assert_eq!(diagnostics.as_ptr(), allocation);
    assert_eq!(diagnostics.capacity(), capacity);
    assert!(state.semantic_diagnostics.is_empty());
    assert_eq!(state.semantic_diagnostics.capacity(), 0);
    assert!(matches!(
        &diagnostics[..],
        [
            CommandSemanticDiagnostic::Trace {
                text,
                force_newline: false,
            },
            CommandSemanticDiagnostic::MissingNumber { context, site: None },
            CommandSemanticDiagnostic::PdfExpansionMessage { text: pdf_text },
        ] if text == "first" && context == "second" && pdf_text == "third"
    ));
}

#[derive(Default)]
struct NamedPushObserver(Vec<CommandObservation>);

impl CommandObserver for NamedPushObserver {
    fn committed(&mut self, observation: CommandObservation) {
        self.0.push(observation);
    }
}

#[test]
fn named_token_list_pushes_publish_directly_to_the_optional_observer_in_order() {
    crate::test_harness::with_universe(|universe| {
        let tokens = universe
            .command_context()
            .expect("named-push token context")
            .allocate_token_list(&[word('x').token_word()])
            .expect("named-push token list");
        let mut state = CommandState::default();
        let context = universe
            .command_context()
            .expect("named-push installation context");
        state.push_everypar(&context, tokens.clone());
        state.push_everymath(&context, tokens, false);
        drop(context);

        let mut observer = NamedPushObserver::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        state.publish_named_token_list_pushes(
            &mut universe
                .command_context()
                .expect("named-push publication context"),
            &mut diagnostic_effects,
            Some(&mut observer),
        );

        let reasons: Vec<_> = observer
            .0
            .iter()
            .map(|observation| match observation {
                CommandObservation::Input(record) => {
                    assert_eq!(record.transition, InputTransition::Push);
                    record.reason
                }
                other => panic!("named push emitted non-input observation: {other:?}"),
            })
            .collect();
        assert_eq!(reasons, [InputReason::EveryPar, InputReason::EveryMath]);
        assert!(state.named_token_list_pushes.is_empty());
    });
}

#[cfg(feature = "profiling")]
#[test]
fn unobserved_named_token_list_publication_allocates_nothing_at_depth_4096() {
    crate::test_harness::with_universe(|universe| {
        let tokens = universe
            .command_context()
            .expect("unobserved named-push token context")
            .allocate_token_list(&[word('x').token_word()])
            .expect("unobserved named-push token list");
        let mut state = CommandState::default();
        let context = universe
            .command_context()
            .expect("unobserved named-push installation context");
        for _ in 0..4_096 {
            state.push_everypar(&context, tokens.clone());
        }
        drop(context);

        let owner = tex_state::measurement::HotCoreAllocationOwner::EvidencePublication;
        let before = tex_state::measurement::hot_core_thread_allocation_measurement(owner);
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        {
            let _scope = tex_state::measurement::hot_core_allocation_scope(owner);
            state.publish_named_token_list_pushes(
                &mut universe
                    .command_context()
                    .expect("unobserved named-push publication context"),
                &mut diagnostic_effects,
                None,
            );
        }
        let after = tex_state::measurement::hot_core_thread_allocation_measurement(owner);

        assert_eq!(after.calls - before.calls, 0);
        assert_eq!(after.requested_bytes - before.requested_bytes, 0);
        assert!(state.named_token_list_pushes.is_empty());
    });
}

#[test]
fn synchronous_attempt_child_scope_reclaims_only_its_exact_suffix() {
    let mut state = CommandState::<()>::default();
    let child = state.begin_attempt_child_scope();
    let scratch = state
        .attempt
        .allocate_token_list([word('x')])
        .expect("child scratch");
    assert_eq!(
        state.attempt_token_words(scratch).expect("scratch words"),
        &[word('x')]
    );

    state
        .close_attempt_child_scope(child)
        .expect("move-only receipt closes its exact child");
    assert_eq!(
        state.attempt_token_words(scratch),
        Err(AttemptError::InvalidCoordinate)
    );
    state.settle_attempt();
}

#[test]
fn warmed_parameterless_macro_rows_copy_only_compact_definition_keys() {
    crate::test_harness::with_universe(|universe| {
        let definition = universe
            .allocate_definition(&[], &[])
            .expect("empty macro definition");
        let definition_publication = definition;
        let definition = definition_publication.reference();
        let name = universe.intern("ownerprobe").expect("macro name").symbol();
        let context = universe.command_context().expect("command context");
        let mut state = CommandState::default();

        let mut run = |activations: u64| {
            for _ in 0..activations {
                let body = context
                    .admit_macro_body(definition)
                    .expect("resident macro body");
                let level = state.push_macro_activation(name, body, None, OriginId::UNKNOWN);
                let (arguments, replacement) = state
                    .input
                    .levels
                    .innermost_macro_activation()
                    .expect("live macro body");
                assert!(arguments.is_empty() && replacement.is_empty());
                assert!(state.scratch.is_quiescent());

                assert!(state.output_open_context(&context).contains("ownerprobe"));

                state
                    .retire_exhausted_input(level)
                    .expect("empty macro body retirement");
            }
        };

        // Warm every reusable activation/input/scratch lane before measuring.
        run(1);
        run(1);
        run(4_096);
    });
}

#[test]
fn alignment_state_restores_outer_running_depth_after_nested_lifecycle() {
    crate::test_harness::with_universe(|_universe| {
        let mut state = CommandState::<()>::default();
        let outer = AlignmentIdentity::new(1);
        let inner = AlignmentIdentity::new(2);
        state.begin_alignment(outer);
        state.suspend_alignment(outer).expect("suspend outer");
        state.begin_alignment(inner);
        state.finish_alignment(inner).expect("finish inner");
        state.resume_alignment(outer).expect("resume outer");
        state.finish_alignment(outer).expect("finish outer");
        assert_eq!(state.alignment.align_state, 1_000_000);
    });
}

#[test]
fn default_command_state_is_quiescent_at_a_cold_summary_boundary() {
    crate::test_harness::with_universe(|_universe| {
        let state = CommandState::<()>::default();
        assert!(state.scanner.is_quiescent());
        assert!(state.input.levels.is_empty());
    });
}

#[test]
fn native_groups_yield_aftergroup_words_in_unsave_order() {
    crate::test_harness::with_universe(|universe| {
        let mut command = CommandState::default();
        let mut state = universe.command_context().expect("admitted state");
        state
            .begin_group(tex_state::GroupKind::Simple, 1)
            .expect("outer group");
        command
            .save_aftergroup(&mut state, word('a'))
            .expect("outer a");
        command
            .save_aftergroup(&mut state, word('b'))
            .expect("outer b");
        state
            .begin_group(tex_state::GroupKind::Math, 2)
            .expect("inner group");
        command
            .save_aftergroup(&mut state, word('c'))
            .expect("inner c");
        command
            .save_aftergroup(&mut state, word('d'))
            .expect("inner d");
        let mut words = Vec::new();
        state
            .end_group_observed(tex_state::GroupKind::Math, |_, event| {
                if let tex_state::GroupUnsaveEvent::AfterGroup(word) = event {
                    words.push(word);
                }
            })
            .expect("inner close");
        assert_eq!(words, [word('d'), word('c')]);
        assert_eq!(state.execution_group_depth(), 1);
        words.clear();
        state
            .end_group_observed(tex_state::GroupKind::Simple, |_, event| {
                if let tex_state::GroupUnsaveEvent::AfterGroup(word) = event {
                    words.push(word);
                }
            })
            .expect("outer close");
        assert_eq!(words, [word('b'), word('a')]);
        assert_eq!(state.execution_group_depth(), 0);
    });
}
