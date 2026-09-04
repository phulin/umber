use tex_state::glue::{GlueSpec, Order};
use tex_state::provenance::OriginRecord;
use tex_state::scaled::Scaled;
use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};
use tex_state::{DefinitionRef, GlueId, ProvenanceId, TokenListId};

use super::{CommandGroupError, CommandSemanticDiagnostic, CommandState};
use crate::execution_scratch::ArgumentSetId;
use crate::processor::AlignmentIdentity;
use crate::token_collector::ClassifiedToken;
use crate::{
    AttemptDefinitionId, AttemptError, AttemptGlueId, AttemptPromotionDestination,
    AttemptProvenanceId, AttemptTokenListId, CommandObservation, CommandObserver, InputReason,
    InputTransition,
};

enum ResidentRoot<Attempt, Durable> {
    Attempt(Attempt),
    Durable(Durable),
}

fn admit_braced_macro_frame<G>(state: &mut CommandState<G>) -> ArgumentSetId<G> {
    let matching = state.scratch.begin_macro_match().expect("macro match");
    let mut writer = state
        .scratch
        .begin_argument_writer(&matching)
        .expect("argument writer");
    for (token, depth) in [
        (
            Token::Char {
                ch: '[',
                cat: Catcode::BeginGroup,
            },
            1,
        ),
        (
            Token::Char {
                ch: 'x',
                cat: Catcode::Other,
            },
            1,
        ),
        (
            Token::Char {
                ch: ']',
                cat: Catcode::EndGroup,
            },
            0,
        ),
    ] {
        let word = TracedTokenWord::pack(token, OriginId::UNKNOWN);
        assert_eq!(
            state
                .scratch
                .append_argument_token(&mut writer, ClassifiedToken::from_word(word, None), true)
                .expect("argument append"),
            depth
        );
    }
    state
        .scratch
        .publish_argument(writer)
        .expect("argument publication");
    state
        .scratch
        .commit_macro_match(matching)
        .expect("sealed macro frame")
}

struct ResidentPromotion<G> {
    tokens: Vec<ResidentRoot<AttemptTokenListId, TokenListId<G>>>,
    glue: Vec<ResidentRoot<AttemptGlueId, GlueId<G>>>,
    definitions: Vec<ResidentRoot<AttemptDefinitionId, DefinitionRef<G>>>,
    provenance: Vec<ResidentRoot<AttemptProvenanceId, ProvenanceId<G>>>,
}

impl<G> ResidentPromotion<G> {
    fn new(
        tokens: &[AttemptTokenListId],
        glue: &[AttemptGlueId],
        definitions: &[AttemptDefinitionId],
        provenance: &[AttemptProvenanceId],
    ) -> Self {
        Self {
            tokens: tokens.iter().copied().map(ResidentRoot::Attempt).collect(),
            glue: glue.iter().copied().map(ResidentRoot::Attempt).collect(),
            definitions: definitions
                .iter()
                .copied()
                .map(ResidentRoot::Attempt)
                .collect(),
            provenance: provenance
                .iter()
                .copied()
                .map(ResidentRoot::Attempt)
                .collect(),
        }
    }

    fn token(&self, index: usize) -> &TokenListId<G> {
        match &self.tokens[index] {
            ResidentRoot::Durable(tokens) => tokens,
            ResidentRoot::Attempt(_) => panic!("token root was not settled"),
        }
    }

    fn glue(&self, index: usize) -> GlueId<G> {
        match &self.glue[index] {
            ResidentRoot::Durable(glue) => *glue,
            ResidentRoot::Attempt(_) => panic!("glue root was not settled"),
        }
    }

    fn definition(&self, index: usize) -> &DefinitionRef<G> {
        match &self.definitions[index] {
            ResidentRoot::Durable(definition) => definition,
            ResidentRoot::Attempt(_) => panic!("definition root was not settled"),
        }
    }

    fn provenance(&self, index: usize) -> ProvenanceId<G> {
        match &self.provenance[index] {
            ResidentRoot::Durable(provenance) => *provenance,
            ResidentRoot::Attempt(_) => panic!("provenance root was not settled"),
        }
    }
}

impl<G> AttemptPromotionDestination<G> for ResidentPromotion<G> {
    fn token_root_count(&self) -> usize {
        self.tokens.len()
    }

    fn token_root(&self, index: usize) -> AttemptTokenListId {
        match self.tokens[index] {
            ResidentRoot::Attempt(root) => root,
            ResidentRoot::Durable(_) => panic!("token root settled before preflight ended"),
        }
    }

    fn next_token_root(&self) -> AttemptTokenListId {
        self.tokens
            .iter()
            .find_map(|root| match root {
                ResidentRoot::Attempt(root) => Some(*root),
                ResidentRoot::Durable(_) => None,
            })
            .expect("token root remains")
    }

    fn settle_token_root(&mut self, source: AttemptTokenListId, tokens: TokenListId<G>) {
        let mut matched = 0;
        for root in &mut self.tokens {
            if matches!(root, ResidentRoot::Attempt(candidate) if *candidate == source) {
                *root = ResidentRoot::Durable(tokens.clone());
                matched += 1;
            }
        }
        assert_ne!(matched, 0);
    }

    fn glue_root_count(&self) -> usize {
        self.glue.len()
    }

    fn glue_root(&self, index: usize) -> AttemptGlueId {
        match self.glue[index] {
            ResidentRoot::Attempt(root) => root,
            ResidentRoot::Durable(_) => panic!("glue root settled before preflight ended"),
        }
    }

    fn next_glue_root(&self) -> AttemptGlueId {
        self.glue
            .iter()
            .find_map(|root| match root {
                ResidentRoot::Attempt(root) => Some(*root),
                ResidentRoot::Durable(_) => None,
            })
            .expect("glue root remains")
    }

    fn settle_glue_root(&mut self, source: AttemptGlueId, glue: GlueId<G>) {
        let mut matched = 0;
        for root in &mut self.glue {
            if matches!(root, ResidentRoot::Attempt(candidate) if *candidate == source) {
                *root = ResidentRoot::Durable(glue);
                matched += 1;
            }
        }
        assert_ne!(matched, 0);
    }

    fn definition_root_count(&self) -> usize {
        self.definitions.len()
    }

    fn definition_root(&self, index: usize) -> AttemptDefinitionId {
        match self.definitions[index] {
            ResidentRoot::Attempt(root) => root,
            ResidentRoot::Durable(_) => panic!("definition root settled before preflight ended"),
        }
    }

    fn next_definition_root(&self) -> AttemptDefinitionId {
        self.definitions
            .iter()
            .find_map(|root| match root {
                ResidentRoot::Attempt(root) => Some(*root),
                ResidentRoot::Durable(_) => None,
            })
            .expect("definition root remains")
    }

    fn settle_definition_root(
        &mut self,
        source: AttemptDefinitionId,
        definition: DefinitionRef<G>,
    ) {
        let mut matched = 0;
        for root in &mut self.definitions {
            if matches!(root, ResidentRoot::Attempt(candidate) if *candidate == source) {
                *root = ResidentRoot::Durable(definition);
                matched += 1;
            }
        }
        assert_ne!(matched, 0);
    }

    fn provenance_root_count(&self) -> usize {
        self.provenance.len()
    }

    fn provenance_root(&self, index: usize) -> AttemptProvenanceId {
        match self.provenance[index] {
            ResidentRoot::Attempt(root) => root,
            ResidentRoot::Durable(_) => panic!("provenance root settled before preflight ended"),
        }
    }

    fn next_provenance_root(&self) -> AttemptProvenanceId {
        self.provenance
            .iter()
            .find_map(|root| match root {
                ResidentRoot::Attempt(root) => Some(*root),
                ResidentRoot::Durable(_) => None,
            })
            .expect("provenance root remains")
    }

    fn settle_provenance_root(&mut self, source: AttemptProvenanceId, provenance: ProvenanceId<G>) {
        let mut matched = 0;
        for root in &mut self.provenance {
            if matches!(root, ResidentRoot::Attempt(candidate) if *candidate == source) {
                *root = ResidentRoot::Durable(provenance);
                matched += 1;
            }
        }
        assert_ne!(matched, 0);
    }
}

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
    fn run(depth: usize) -> ((u64, u64, u64, u64), u64, u64, u64) {
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
            let copies_before = state.profile_timeline_counters().full_frame_history_clones;
            let owner = tex_state::measurement::HotCoreAllocationOwner::DeliveryAndScan;
            let before = tex_state::measurement::hot_core_thread_allocation_measurement(owner);
            {
                let _scope = tex_state::measurement::hot_core_allocation_scope(owner);
                for _ in 0..4_096 {
                    std::hint::black_box(state.current_file_source_id());
                }
            }
            let after = tex_state::measurement::hot_core_thread_allocation_measurement(owner);
            let copies_after = state.profile_timeline_counters().full_frame_history_clones;
            (
                state.profile_input_source_context_counters(),
                after.calls - before.calls,
                after.requested_bytes - before.requested_bytes,
                copies_after - copies_before,
            )
        })
    }

    let shallow = run(1);
    let deep = run(4_096);
    assert_eq!(shallow, ((4_096, 0, 0, 0), 0, 0, 0));
    assert_eq!(deep, shallow);
}

#[test]
#[cfg(feature = "profiling")]
fn source_lexer_mutation_borrows_its_checked_slot_once() {
    crate::test_harness::with_universe(|universe| {
        let mut state = CommandState::default();
        let source = state
            .register_source(crate::SourceRegistration::new(
                crate::RegisteredSourceKind::Generated,
                std::sync::Arc::<[u8]>::from(&b"x"[..]),
            ))
            .expect("source lexer fixture source");
        state
            .open_registered_source(source)
            .expect("source lexer fixture opens");
        state.profile_prepare_source_line(13);
        let summary = state
            .publish_summary(universe)
            .expect("source lexer fixture checkpoint");
        state.profile_repeated_source_lex_mutations(1);
        state
            .restore_summary(&summary, universe)
            .expect("source lexer fixture warm restore");

        state.profile_reset_input_source_context_counters();
        let owner = tex_state::measurement::HotCoreAllocationOwner::DeliveryAndScan;
        let before = tex_state::measurement::hot_core_thread_allocation_measurement(owner);
        {
            let _scope = tex_state::measurement::hot_core_allocation_scope(owner);
            state.profile_repeated_source_lex_mutations(4_096);
        }
        let after = tex_state::measurement::hot_core_thread_allocation_measurement(owner);
        assert_eq!(
            state.profile_input_source_context_counters(),
            (0, 0, 0, 4_096)
        );
        assert_eq!(after.calls - before.calls, 0);
        assert_eq!(after.requested_bytes - before.requested_bytes, 0);
    });
}

fn attempt_definition<G>(
    state: &mut CommandState<G>,
    parameters: &[TracedTokenWord],
    replacement: &[TracedTokenWord],
) -> crate::AttemptDefinitionId {
    let definition = state
        .attempt
        .arena_mut()
        .allocate_definition_builder()
        .expect("definition builder");
    for word in parameters {
        state
            .attempt
            .arena_mut()
            .push_definition_parameter(definition, word.token_word())
            .expect("parameter word");
    }
    state
        .attempt
        .arena_mut()
        .finish_definition_parameters(definition)
        .expect("parameter boundary");
    for word in replacement {
        state
            .attempt
            .arena_mut()
            .push_definition_replacement(definition, word.token_word())
            .expect("replacement word");
    }
    state
        .attempt
        .arena_mut()
        .finish_definition(definition)
        .expect("definition");
    definition
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
    let operation = state.begin_attempt_operation();
    let child = state
        .begin_attempt_child_scope()
        .expect("active operation admits one synchronous child");
    let scratch = state
        .attempt
        .arena_mut()
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
    state
        .commit_attempt_operation(operation)
        .expect("child close left the parent owner intact");
}

#[test]
fn synchronous_attempt_child_scope_requires_an_active_operation() {
    let mut state = CommandState::<()>::default();
    assert_eq!(
        state
            .begin_attempt_child_scope()
            .expect_err("a synchronous child requires an active operation"),
        AttemptError::InvalidCoordinate
    );
    let operation = state.begin_attempt_operation();
    let child = state
        .begin_attempt_child_scope()
        .expect("active operation admits a child");
    state
        .close_attempt_child_scope(child)
        .expect("child closes normally");
    state
        .commit_attempt_operation(operation)
        .expect("parent closes normally");
}

fn glue(width: i32) -> GlueSpec {
    GlueSpec {
        width: Scaled::from_raw(width),
        stretch: Scaled::from_raw(0),
        stretch_order: Order::Normal,
        shrink: Scaled::from_raw(0),
        shrink_order: Order::Normal,
    }
}

#[test]
fn attempt_promotion_preserves_multiple_root_order_and_duplicates() {
    crate::test_harness::with_universe(|universe| {
        let mut state = CommandState::default();
        let first = state
            .attempt
            .arena_mut()
            .allocate_token_list([word('a')])
            .expect("first list");
        let second = state
            .attempt
            .arena_mut()
            .allocate_token_list([word('b')])
            .expect("second list");

        let mut destination = ResidentPromotion::new(&[second, first, second], &[], &[], &[]);
        state
            .promote_attempt_roots_into(universe, &mut destination)
            .expect("promotion");

        assert_eq!(destination.token(0), destination.token(2));
        let admitted = universe.command_context().expect("admission");
        assert_eq!(
            admitted
                .token_list(destination.token(0).clone())
                .iter()
                .collect::<Vec<_>>(),
            &[word('b').token_word()]
        );
        assert_eq!(
            admitted
                .token_list(destination.token(1).clone())
                .iter()
                .collect::<Vec<_>>(),
            &[word('a').token_word()]
        );
    });
}

#[test]
fn attempt_promotion_returns_mixed_roots_in_declared_order() {
    crate::test_harness::with_universe(|universe| {
        let mut state = CommandState::default();
        let replacement = state
            .attempt
            .arena_mut()
            .allocate_token_list([word('x')])
            .expect("replacement text");
        let definition = attempt_definition(&mut state, &[word('#')], &[word('x')]);
        let glue_root = state
            .attempt
            .arena_mut()
            .allocate_glue(glue(42))
            .expect("glue");
        let provenance = state
            .attempt
            .arena_mut()
            .allocate_provenance(OriginRecord::UnknownBootstrap)
            .expect("provenance");

        let mut destination =
            ResidentPromotion::new(&[replacement], &[glue_root], &[definition], &[provenance]);
        state
            .promote_attempt_roots_into(universe, &mut destination)
            .expect("mixed promotion");

        let admitted = universe.command_context().expect("admission");
        assert_eq!(
            admitted
                .token_list(destination.token(0).clone())
                .iter()
                .collect::<Vec<_>>(),
            &[word('x').token_word()]
        );
        assert_eq!(admitted.glue(destination.glue(0)), glue(42));
        assert_eq!(
            admitted
                .definition(*destination.definition(0))
                .replacement_text(),
            &[word('x').token_word()]
        );
        assert_eq!(
            admitted.provenance(destination.provenance(0)),
            OriginRecord::UnknownBootstrap
        );
    });
}

#[cfg(feature = "profiling")]
#[test]
fn warmed_single_definition_promotion_ignores_the_large_live_attempt_arena() {
    crate::test_harness::with_universe(|universe| {
        let mut state = CommandState::default();
        for _ in 0..16_384 {
            state
                .attempt
                .arena_mut()
                .allocate_token_list([word('z')])
                .expect("large unrelated attempt row");
        }

        for _ in 0..17 {
            let definition = attempt_definition(&mut state, &[word('#')], &[word('x')]);
            state
                .promote_attempt_definition(universe, definition)
                .expect("warm definition promotion");
        }
        let definitions: [crate::AttemptDefinitionId; 8] =
            std::array::from_fn(|_| attempt_definition(&mut state, &[word('#')], &[word('x')]));
        let mut published: [Option<_>; 8] = std::array::from_fn(|_| None);
        let owner = tex_state::measurement::HotCoreAllocationOwner::SemanticApply;
        let before = tex_state::measurement::hot_core_thread_allocation_measurement(owner);
        {
            let _scope = tex_state::measurement::hot_core_allocation_scope(owner);
            for (definition, destination) in definitions.into_iter().zip(&mut published) {
                *destination = Some(
                    state
                        .promote_attempt_definition(universe, definition)
                        .expect("measured distinct definition promotion"),
                );
            }
        }
        let after = tex_state::measurement::hot_core_thread_allocation_measurement(owner);
        assert_eq!(after.calls - before.calls, 0);
        assert_eq!(after.requested_bytes - before.requested_bytes, 0);
        let context = universe.command_context().expect("definition context");
        for (index, definition) in published.iter().enumerate() {
            let definition = definition.as_ref().expect("measured publication");
            assert_eq!(
                context.definition(*definition).replacement_text(),
                [word('x').token_word()]
            );
            for other in &published[..index] {
                assert_ne!(
                    definition,
                    other.as_ref().expect("earlier measured publication"),
                    "each measured publication receives its own destination serial"
                );
            }
        }

        let final_definition = published[7].take().expect("last durable definition");
        assert_eq!(
            context.definition(final_definition).replacement_text(),
            [word('x').token_word()],
            "dropping the other durable ids cannot release this definition"
        );
    });
}

#[cfg(feature = "profiling")]
#[test]
fn one_and_4096_resident_promotions_use_bounded_region_growth_and_keep_owners_stationary() {
    fn evidence(
        repetitions: usize,
    ) -> (
        tex_state::measurement::HotCoreAllocationMeasurement,
        usize,
        u64,
    ) {
        crate::test_harness::with_universe(|universe| {
            let mut state = CommandState::default();

            let warm: Vec<_> = (0..repetitions)
                .map(|_| attempt_definition(&mut state, &[], &[word('w')]))
                .collect();
            let mut warm_destination = ResidentPromotion::new(&[], &[], &warm, &[]);
            state
                .promote_attempt_roots_into(universe, &mut warm_destination)
                .expect("warm resident promotion");
            drop(warm_destination);

            let definitions: Vec<_> = (0..repetitions)
                .map(|_| attempt_definition(&mut state, &[], &[word('x')]))
                .collect();
            let mut destination = ResidentPromotion::new(&[], &[], &definitions, &[]);
            let state_address = std::ptr::from_ref(&state);
            let attempt_address = std::ptr::from_ref(state.attempt.arena());
            let roots_address = destination.definitions.as_ptr();
            let owner = tex_state::measurement::HotCoreAllocationOwner::SemanticApply;
            let before = tex_state::measurement::hot_core_thread_allocation_measurement(owner);
            let trace_start = tex_state::measurement::hot_core_allocation_trace_cursor();
            {
                let _scope = tex_state::measurement::hot_core_allocation_scope(owner);
                state
                    .promote_attempt_roots_into(universe, &mut destination)
                    .expect("measured resident promotion");
            }
            let after = tex_state::measurement::hot_core_thread_allocation_measurement(owner);
            let trace_end = tex_state::measurement::hot_core_allocation_trace_cursor();
            let trace_entries = (trace_start..trace_end)
                .filter_map(tex_state::measurement::hot_core_allocation_trace_entry)
                .filter(|entry| entry.owner == owner)
                .collect::<Vec<_>>();
            assert_eq!(
                trace_entries.len(),
                (after.calls - before.calls) as usize,
                "every measured semantic-apply allocation has one trace entry"
            );
            if repetitions == 4_096 {
                let payload_bytes = 4_096 * std::mem::size_of::<tex_state::token::TokenWord>();
                let combined_chunk_lower_bound = payload_bytes + 2 * std::mem::size_of::<usize>();
                assert!(
                    trace_entries.iter().any(|entry| {
                        entry.requested_bytes >= combined_chunk_lower_bound
                            && entry.requested_bytes
                                <= combined_chunk_lower_bound + std::mem::size_of::<usize>()
                    }),
                    "the overflow payload and Rc header must share one allocation: {trace_entries:?}"
                );
                assert!(
                    !trace_entries
                        .iter()
                        .any(|entry| entry.requested_bytes == payload_bytes),
                    "no standalone fixed overflow payload allocation: {trace_entries:?}"
                );
            }

            let address_changes = usize::from(std::ptr::from_ref(&state) != state_address)
                + usize::from(std::ptr::from_ref(state.attempt.arena()) != attempt_address)
                + usize::from(destination.definitions.as_ptr() != roots_address);
            let admitted = universe.command_context().expect("definition context");
            let mut checksum = 0_u64;
            for index in 0..repetitions {
                let definition = admitted.definition(*destination.definition(index));
                assert_eq!(definition.replacement_text(), [word('x').token_word()]);
                checksum ^= u64::from(
                    definition
                        .replacement_text()
                        .get(0)
                        .expect("replacement word")
                        .raw(),
                )
                .rotate_left((index & 63) as u32);
            }
            (
                tex_state::measurement::HotCoreAllocationMeasurement {
                    calls: after.calls - before.calls,
                    requested_bytes: after.requested_bytes - before.requested_bytes,
                },
                address_changes,
                std::hint::black_box(checksum),
            )
        })
    }

    let (one_allocations, one_address_changes, one_checksum) = evidence(1);
    let (many_allocations, many_address_changes, many_checksum) = evidence(4_096);
    assert_eq!(one_allocations.calls, 0);
    assert_eq!(one_allocations.requested_bytes, 0);
    assert_eq!(
        many_allocations.calls, 4,
        "one word chunk, its flat directory, and the live/owner header directories grow"
    );
    assert!(many_allocations.requested_bytes > 0);
    assert_eq!(one_address_changes, 0);
    assert_eq!(many_address_changes, 0);
    assert_ne!(one_checksum, many_checksum);
}

#[test]
fn foreign_attempt_root_rejection_is_mutation_free() {
    crate::test_harness::with_universe(|universe| {
        let mut state = CommandState::<_>::default();
        let mut foreign = CommandState::<()>::default();
        let foreign_root = foreign
            .attempt
            .arena_mut()
            .allocate_token_list([word('x')])
            .expect("foreign root");

        let mut destination = ResidentPromotion::new(&[foreign_root], &[], &[], &[]);
        assert!(matches!(
            state.promote_attempt_roots_into(universe, &mut destination),
            Err(AttemptError::ForeignAttempt)
        ));
        let retirement = universe.retire().expect("retirement");
        assert_eq!(retirement.token_list_rows(), 0);
        assert_eq!(retirement.definition_rows(), 0);
        assert_eq!(retirement.glue_rows(), 0);
        assert_eq!(retirement.provenance_rows(), 0);
    });
}

#[test]
fn stale_root_rejection_validates_complete_batch_before_mutation() {
    crate::test_harness::with_universe(|universe| {
        let mut state = CommandState::default();
        let valid = state
            .attempt
            .arena_mut()
            .allocate_token_list([word('a')])
            .expect("valid root");
        let mark = state.begin_attempt_operation();
        let stale = state
            .attempt
            .arena_mut()
            .allocate_token_list([word('b')])
            .expect("stale root");
        state
            .rollback_attempt_operation(mark)
            .expect("operation rolls back");

        let mut destination = ResidentPromotion::new(&[valid, stale], &[], &[], &[]);
        assert!(matches!(
            state.promote_attempt_roots_into(universe, &mut destination),
            Err(AttemptError::InvalidCoordinate)
        ));
        let retirement = universe.retire().expect("retirement");
        assert_eq!(retirement.token_list_rows(), 0);
        assert_eq!(retirement.definition_rows(), 0);
        assert_eq!(retirement.glue_rows(), 0);
        assert_eq!(retirement.provenance_rows(), 0);
    });
}

#[test]
fn operation_discard_truncates_only_the_attempt_suffix() {
    crate::test_harness::with_universe(|_universe| {
        let mut state = CommandState::<()>::default();
        let retained = state
            .attempt
            .arena_mut()
            .allocate_token_list([word('a')])
            .expect("retained list");
        let mark = state.begin_attempt_operation();
        let rejected = state
            .attempt
            .arena_mut()
            .allocate_token_list([word('b')])
            .expect("candidate list");

        state
            .rollback_attempt_operation(mark)
            .expect("operation rolls back");
        assert_eq!(
            state.attempt_token_words(retained).expect("retained words"),
            &[word('a')]
        );
        assert!(state.attempt_token_words(rejected).is_err());
    });
}

#[test]
fn operation_rollback_discards_nested_brace_argument_facts_with_the_frame() {
    let mut state = CommandState::<()>::default();
    let operation = state.begin_attempt_operation();
    let _frame = admit_braced_macro_frame(&mut state);
    assert_eq!(state.scratch.frame_len(), 1);

    state
        .rollback_attempt_operation(operation)
        .expect("rollback retires operation-local macro frames");
    assert!(state.scratch.is_quiescent());
}

#[test]
fn successful_scope_commit_reclaims_promoted_operation_rows() {
    crate::test_harness::with_universe(|universe| {
        let mut state = CommandState::default();
        let operation = state.begin_attempt_operation();
        let definition = attempt_definition(&mut state, &[word('#')], &[word('x')]);
        let durable = state
            .promote_attempt_definition(universe, definition)
            .expect("successful operation publishes its durable root");

        state
            .commit_attempt_operation(operation)
            .expect("operation scope commits");
        assert!(state.attempt.is_empty());
        assert_eq!(
            universe
                .command_context()
                .expect("admission")
                .definition(durable)
                .replacement_text(),
            &[word('x').token_word()]
        );
    });
}

#[test]
fn ordinary_attempt_lifecycle_has_one_coordinate_owner_and_a_coordinate_free_edge() {
    assert!(core::mem::size_of::<crate::CommandAttemptMark>() > 0);
    assert_eq!(
        core::mem::size_of::<crate::CommandAttemptOperation>(),
        0,
        "the move-only executor edge must carry no ordinary coordinate"
    );

    let mut state = CommandState::<()>::default();
    let operation = state.begin_attempt_operation();
    assert!(state.active_attempt_operation.is_some());
    state
        .commit_attempt_operation(operation)
        .expect("the state-owned coordinate commits");
    assert!(state.active_attempt_operation.is_none());
}

#[test]
#[cfg(feature = "profiling")]
fn warmed_attempt_lifecycle_allocates_nothing() {
    let mut state = CommandState::<()>::default();
    let warm = state.begin_attempt_operation();
    state
        .commit_attempt_operation(warm)
        .expect("warm operation commits");

    let owner = tex_state::measurement::HotCoreAllocationOwner::DeliveryAndScan;
    let before = tex_state::measurement::hot_core_thread_allocation_measurement(owner);
    {
        let _scope = tex_state::measurement::hot_core_allocation_scope(owner);
        for _ in 0..4_096 {
            let operation = state.begin_attempt_operation();
            std::hint::black_box(&state.active_attempt_operation);
            state
                .commit_attempt_operation(operation)
                .expect("measured operation commits");
        }
    }
    let after = tex_state::measurement::hot_core_thread_allocation_measurement(owner);
    assert_eq!(after.calls - before.calls, 0);
    assert_eq!(after.requested_bytes - before.requested_bytes, 0);
}

#[test]
fn macro_scratch_descriptor_survives_attempt_suspension_without_an_arena_owner() {
    crate::test_harness::with_universe(|universe| {
        let definition = universe
            .allocate_definition(&[], &[])
            .expect("empty macro definition");
        let name = universe
            .intern("suspendedmacro")
            .expect("macro name")
            .symbol();
        let mut state = CommandState::default();
        let operation = state.begin_attempt_operation();
        let frame = admit_braced_macro_frame(&mut state);
        let body = universe
            .command_context()
            .expect("command context")
            .admit_macro_body(definition)
            .expect("resident macro body")
            .2;
        let level = state.push_macro_activation(name, body, Some(frame), OriginId::UNKNOWN);

        let pending = state
            .suspend_attempt(
                universe,
                operation,
                crate::AttemptResumePoint::default(),
                "resource",
            )
            .expect("attempt suspension");
        assert!(state.attempt.is_empty());
        assert_eq!(state.scratch.frame_len(), 1);
        let (resumed, _, request) = state
            .resume_attempt(universe, pending)
            .ok()
            .expect("attempt resumption");
        assert_eq!(request, "resource");
        let range = state
            .scratch
            .argument_range(frame, 1)
            .expect("resumed macro frame")
            .expect("resumed argument");
        assert!(
            state
                .scratch
                .argument_facts(range)
                .expect("resumed argument facts")
                .removable_outer_group()
        );
        state
            .retire_exhausted_input(level)
            .expect("macro body retirement");
        assert!(state.scratch.is_quiescent());
        state
            .commit_attempt_operation(resumed)
            .expect("operation commit");
    });
}

#[test]
fn warmed_parameterless_macro_rows_copy_only_compact_definition_keys() {
    crate::test_harness::with_universe(|universe| {
        let definition = universe
            .allocate_definition(&[], &[])
            .expect("empty macro definition");
        let name = universe.intern("ownerprobe").expect("macro name").symbol();
        let context = universe.command_context().expect("command context");
        let mut state = CommandState::default();

        let mut run = |activations: u64| {
            let retained_before = tex_state::definition_retain_count();
            for _ in 0..activations {
                let level = state.push_macro_activation(
                    name,
                    context
                        .admit_macro_body(definition)
                        .expect("resident macro body")
                        .2,
                    None,
                    OriginId::UNKNOWN,
                );
                let body = state
                    .input
                    .levels
                    .last()
                    .and_then(crate::input::InputLevel::macro_body)
                    .expect("live macro body");
                assert_eq!(body.body.definition_ref().semantic_owner_count(), 0);
                assert!(
                    state
                        .input
                        .levels
                        .last()
                        .is_some_and(|level| level.macro_body().is_some())
                );
                assert_eq!(state.scratch.frame_len(), 0);
                let retained_before_context = tex_state::definition_retain_count();
                assert!(state.output_open_context(&context).contains("ownerprobe"));
                assert_eq!(
                    tex_state::definition_retain_count(),
                    retained_before_context,
                    "context projection borrows the activation definition"
                );
                state
                    .retire_exhausted_input(level)
                    .expect("empty macro body retirement");
                assert_eq!(definition.semantic_owner_count(), 0);
            }
            assert_eq!(
                tex_state::definition_retain_count() - retained_before,
                0,
                "compact definition keys do not retain per-definition owners"
            );
        };

        // Warm every reusable activation/input/scratch lane before measuring.
        run(1);
        run(1);
        run(4_096);
    });
}

#[test]
fn resource_suspension_moves_the_arena_and_restores_its_opening_cursor() {
    crate::test_harness::with_universe(|universe| {
        let mut state = CommandState::default();
        let retained = state
            .attempt
            .arena_mut()
            .allocate_token_list([word('a')])
            .expect("pre-operation attempt value");
        let opening = state.begin_attempt_operation();
        let rejected = state
            .attempt
            .arena_mut()
            .allocate_token_list([word('b')])
            .expect("operation-local attempt value");
        let resume = crate::AttemptResumePoint {
            command: 3,
            scanner: 5,
            expansion: 7,
            subordinate: 11,
        };

        let pending = state
            .suspend_attempt(universe, opening, resume, "font request")
            .expect("live generation owner");
        assert!(state.attempt.is_empty());
        assert_eq!(
            universe.retire(),
            Err(tex_state::UniverseError::State(
                tex_state::StateError::GenerationInUse
            ))
        );

        let (restored_opening, restored_resume, request) = state
            .resume_attempt(universe, pending)
            .ok()
            .expect("same admitted generation");
        assert_eq!(restored_resume, resume);
        assert_eq!(request, "font request");
        state
            .rollback_attempt_operation(restored_opening)
            .expect("resumed operation rolls back");
        assert_eq!(
            state.attempt_token_words(retained).expect("retained words"),
            &[word('a')]
        );
        assert!(state.attempt_token_words(rejected).is_err());
    });
}

#[test]
fn nested_scanner_scopes_survive_resource_suspension_and_resume_once() {
    crate::test_harness::with_universe(|universe| {
        let mut state = CommandState::default();
        let operation = state.begin_attempt_operation();
        let scanner = state.begin_attempt_scanner_scope().expect("scanner scope");
        let scanner_child = state.begin_attempt_scanner_scope().expect("scanner child");
        let child_value = state
            .attempt
            .arena_mut()
            .allocate_token_list([word('x')])
            .expect("nested child value");
        let pending = state
            .suspend_attempt(
                universe,
                operation,
                crate::AttemptResumePoint::default(),
                (scanner, scanner_child),
            )
            .expect("nested scopes suspend with their arena owner");

        let (resumed, _, (scanner, scanner_child)) = state
            .resume_attempt(universe, pending)
            .ok()
            .expect("nested scopes resume into the same state");
        assert_eq!(
            state.attempt_token_words(child_value).expect("child words"),
            &[word('x')]
        );
        state
            .discard_attempt_scope_suffix(scanner_child)
            .expect("top scanner child retires");
        state
            .defer_attempt_scope_retirement(scanner)
            .expect("scanner defers until commit");
        state
            .commit_attempt_operation(resumed)
            .expect("commit consumes each owner exactly once");
        assert!(state.attempt.is_empty());
    });
}

#[test]
fn failed_resource_suspension_keeps_the_live_attempt_installed() {
    crate::test_harness::with_universe(|universe| {
        let mut state = CommandState::default();
        let retained = state
            .attempt
            .arena_mut()
            .allocate_token_list([word('x')])
            .expect("attempt value");
        let opening = state.begin_attempt_operation();
        universe.retire().expect("unowned generation retires");

        let failure = match state.suspend_attempt(
            universe,
            opening,
            crate::AttemptResumePoint::default(),
            "input request",
        ) {
            Ok(_) => panic!("retired generation must reject suspension"),
            Err(failure) => failure,
        };
        let (opening, error) = failure.into_parts();
        assert!(matches!(error, crate::AttemptSuspendError::Generation(_)));
        assert_eq!(
            state.attempt_token_words(retained).expect("retained words"),
            &[word('x')]
        );
        state
            .commit_attempt_operation(opening)
            .expect("rejected suspension returns the live operation owner");
    });
}

#[test]
fn suspension_requires_a_live_state_owned_coordinate() {
    crate::test_harness::with_universe(|universe| {
        let mut state = CommandState::default();
        let retained = state
            .attempt
            .arena_mut()
            .allocate_token_list([word('x')])
            .expect("retained attempt value");
        let mut foreign = CommandState::<()>::default();
        let foreign_operation = foreign.begin_attempt_operation();

        let failure = match state.suspend_attempt(
            universe,
            foreign_operation,
            crate::AttemptResumePoint::default(),
            "input request",
        ) {
            Ok(_) => panic!("a capability cannot supply another state's missing coordinate"),
            Err(failure) => failure,
        };
        let (foreign_operation, error) = failure.into_parts();
        assert!(matches!(
            error,
            crate::AttemptSuspendError::StaleMark(crate::AttemptError::InvalidCoordinate)
        ));
        assert_eq!(
            state.attempt_token_words(retained).expect("retained words"),
            &[word('x')]
        );

        foreign
            .rollback_attempt_operation(foreign_operation)
            .expect("rejected suspension returns the foreign lifecycle edge");
    });
}

#[test]
fn resource_resume_rejects_a_nonempty_live_attempt_without_mutation() {
    crate::test_harness::with_universe(|universe| {
        let mut state = CommandState::default();
        let opening = state.begin_attempt_operation();
        let pending = state
            .suspend_attempt(
                universe,
                opening,
                crate::AttemptResumePoint::default(),
                "font request",
            )
            .expect("attempt suspends");
        let live = state
            .attempt
            .arena_mut()
            .allocate_token_list([word('z')])
            .expect("new live attempt value");

        let pending = state
            .resume_attempt(universe, pending)
            .expect_err("a pending arena cannot overwrite live attempt state");
        assert_eq!(
            state.attempt_token_words(live).expect("live words"),
            &[word('z')]
        );

        state.attempt = crate::CommandAttempt::default();
        let (_, _, request) = state
            .resume_attempt(universe, pending)
            .ok()
            .expect("unchanged pending attempt remains resumable");
        assert_eq!(request, "font request");
    });
}

#[test]
fn resource_resume_rejects_a_wrong_pending_coordinate_without_mutation() {
    crate::test_harness::with_universe(|universe| {
        let mut source = CommandState::default();
        let operation = source.begin_attempt_operation();
        let pending = source
            .suspend_attempt(
                universe,
                operation,
                crate::AttemptResumePoint::default(),
                "input request",
            )
            .expect("source attempt suspends");

        let mut destination = CommandState::default();
        let destination_operation = destination.begin_attempt_operation();
        destination.attempt = crate::CommandAttempt::default();
        let pending = destination
            .resume_attempt(universe, pending)
            .expect_err("the cold coordinate rejects another command state's admission");
        assert!(destination.attempt.is_empty());

        let (resumed, _, request) = source
            .resume_attempt(universe, pending)
            .ok()
            .expect("rejected pending continuation remains resumable by its owner");
        assert_eq!(request, "input request");
        source
            .commit_attempt_operation(resumed)
            .expect("source operation commits");
        drop(destination_operation);
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
fn nested_group_payloads_restore_exact_save_order() {
    crate::test_harness::with_universe(|universe| {
        let mut command = CommandState::default();
        let mut state = universe.command_context().expect("admitted state");
        command
            .begin_group(&mut state, tex_state::GroupKind::Simple, 1)
            .expect("outer group");
        command.save_aftergroup(&state, word('a')).expect("outer a");
        command.save_aftergroup(&state, word('b')).expect("outer b");
        let outer_projection = command.aftergroup_save_stack_projection();
        assert_eq!(outer_projection.0, 2);
        assert!(outer_projection.1.is_some());
        command
            .begin_group(&mut state, tex_state::GroupKind::Math, 2)
            .expect("inner group");
        command.save_aftergroup(&state, word('c')).expect("inner c");
        command.save_aftergroup(&state, word('d')).expect("inner d");
        let nested_projection = command.aftergroup_save_stack_projection();
        assert_eq!(nested_projection.0, 4);
        assert!(nested_projection.1 > outer_projection.1);

        assert_eq!(
            command
                .end_group(&mut state, tex_state::GroupKind::Math)
                .expect("inner closes")
                .into_aftergroup(),
            vec![word('c'), word('d')]
        );
        assert_eq!(
            command.aftergroup_save_stack_projection(),
            outer_projection,
            "closing the inner level restores the outer ordering owner"
        );
        assert_eq!(
            command
                .end_group(&mut state, tex_state::GroupKind::Simple)
                .expect("outer closes")
                .into_aftergroup(),
            vec![word('a'), word('b')]
        );
    });
}

#[test]
fn stale_state_group_rejection_precedes_payload_mutation() {
    crate::test_harness::with_universe(|universe| {
        let mut command = CommandState::default();
        let mut state = universe.command_context().expect("admitted state");
        state
            .begin_group(tex_state::GroupKind::Simple, 1)
            .expect("bypass creates stale state");

        assert_eq!(
            command.set_afterassignment(&state, word('x')),
            Err(CommandGroupError::StaleGroupState)
        );
        assert!(!command.has_afterassignment());
    });
}
