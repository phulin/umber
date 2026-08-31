use tex_state::glue::{GlueSpec, Order};
use tex_state::interner::InternerBudget;
use tex_state::scaled::Scaled;
use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};
use tex_state::{
    DefinitionBuildError, DefinitionId, DefinitionIdentityPolicy, GlueId, ProvenanceId, TokenListId,
};

use super::{
    AttemptArena, AttemptDefinitionId, AttemptError, AttemptGlueId, AttemptPromotionDestination,
    AttemptProvenanceId, AttemptResumePoint, AttemptScopeSerial, AttemptTokenListId,
    AttemptTokenStorage, CommandAttempt, PendingCommandAttempt,
};

fn word(ch: char) -> TracedTokenWord {
    TracedTokenWord::pack(
        Token::Char {
            ch,
            cat: Catcode::Other,
        },
        OriginId::UNKNOWN,
    )
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

fn budget() -> InternerBudget {
    InternerBudget::new(64, 64, 4096).expect("test budget")
}

enum TestRoot<Attempt, Durable> {
    Attempt(Attempt),
    Durable(Durable),
}

struct TestPromotionDestination<G> {
    token_lists: Vec<TestRoot<AttemptTokenListId, TokenListId<G>>>,
    glue: Vec<TestRoot<AttemptGlueId, GlueId<G>>>,
    definitions: Vec<TestRoot<AttemptDefinitionId, DefinitionId<G>>>,
    provenance: Vec<TestRoot<AttemptProvenanceId, ProvenanceId<G>>>,
}

impl<G> TestPromotionDestination<G> {
    fn new(
        token_lists: &[AttemptTokenListId],
        glue: &[AttemptGlueId],
        definitions: &[AttemptDefinitionId],
        provenance: &[AttemptProvenanceId],
    ) -> Self {
        Self {
            token_lists: token_lists.iter().copied().map(TestRoot::Attempt).collect(),
            glue: glue.iter().copied().map(TestRoot::Attempt).collect(),
            definitions: definitions.iter().copied().map(TestRoot::Attempt).collect(),
            provenance: provenance.iter().copied().map(TestRoot::Attempt).collect(),
        }
    }

    fn token_list(&self, index: usize) -> &TokenListId<G> {
        match &self.token_lists[index] {
            TestRoot::Durable(tokens) => tokens,
            TestRoot::Attempt(_) => panic!("token root was not settled"),
        }
    }

    fn glue(&self, index: usize) -> GlueId<G> {
        match self.glue[index] {
            TestRoot::Durable(glue) => glue,
            TestRoot::Attempt(_) => panic!("glue root was not settled"),
        }
    }

    fn definition(&self, index: usize) -> &DefinitionId<G> {
        match &self.definitions[index] {
            TestRoot::Durable(definition) => definition,
            TestRoot::Attempt(_) => panic!("definition root was not settled"),
        }
    }

    fn provenance(&self, index: usize) -> ProvenanceId<G> {
        match self.provenance[index] {
            TestRoot::Durable(provenance) => provenance,
            TestRoot::Attempt(_) => panic!("provenance root was not settled"),
        }
    }
}

impl<G> AttemptPromotionDestination<G> for TestPromotionDestination<G> {
    fn token_root_count(&self) -> usize {
        self.token_lists.len()
    }

    fn token_root(&self, index: usize) -> AttemptTokenListId {
        match self.token_lists[index] {
            TestRoot::Attempt(root) => root,
            TestRoot::Durable(_) => panic!("token root settled before preflight ended"),
        }
    }

    fn next_token_root(&self) -> AttemptTokenListId {
        self.token_lists
            .iter()
            .find_map(|root| match root {
                TestRoot::Attempt(root) => Some(*root),
                TestRoot::Durable(_) => None,
            })
            .expect("token root remains")
    }

    fn settle_token_root(&mut self, source: AttemptTokenListId, tokens: TokenListId<G>) {
        let mut matched = 0;
        for root in &mut self.token_lists {
            if matches!(root, TestRoot::Attempt(candidate) if *candidate == source) {
                *root = TestRoot::Durable(tokens.clone());
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
            TestRoot::Attempt(root) => root,
            TestRoot::Durable(_) => panic!("glue root settled before preflight ended"),
        }
    }

    fn next_glue_root(&self) -> AttemptGlueId {
        self.glue
            .iter()
            .find_map(|root| match root {
                TestRoot::Attempt(root) => Some(*root),
                TestRoot::Durable(_) => None,
            })
            .expect("glue root remains")
    }

    fn settle_glue_root(&mut self, source: AttemptGlueId, glue: GlueId<G>) {
        let mut matched = 0;
        for root in &mut self.glue {
            if matches!(root, TestRoot::Attempt(candidate) if *candidate == source) {
                *root = TestRoot::Durable(glue);
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
            TestRoot::Attempt(root) => root,
            TestRoot::Durable(_) => panic!("definition root settled before preflight ended"),
        }
    }

    fn next_definition_root(&self) -> AttemptDefinitionId {
        self.definitions
            .iter()
            .find_map(|root| match root {
                TestRoot::Attempt(root) => Some(*root),
                TestRoot::Durable(_) => None,
            })
            .expect("definition root remains")
    }

    fn settle_definition_root(&mut self, source: AttemptDefinitionId, definition: DefinitionId<G>) {
        let mut matched = 0;
        for root in &mut self.definitions {
            if matches!(root, TestRoot::Attempt(candidate) if *candidate == source) {
                *root = TestRoot::Durable(definition.clone());
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
            TestRoot::Attempt(root) => root,
            TestRoot::Durable(_) => panic!("provenance root settled before preflight ended"),
        }
    }

    fn next_provenance_root(&self) -> AttemptProvenanceId {
        self.provenance
            .iter()
            .find_map(|root| match root {
                TestRoot::Attempt(root) => Some(*root),
                TestRoot::Durable(_) => None,
            })
            .expect("provenance root remains")
    }

    fn settle_provenance_root(&mut self, source: AttemptProvenanceId, provenance: ProvenanceId<G>) {
        let mut matched = 0;
        for root in &mut self.provenance {
            if matches!(root, TestRoot::Attempt(candidate) if *candidate == source) {
                *root = TestRoot::Durable(provenance);
                matched += 1;
            }
        }
        assert_ne!(matched, 0);
    }
}

fn definition<G>(
    attempt: &mut AttemptArena<G>,
    parameters: &[TracedTokenWord],
    replacement: &[TracedTokenWord],
) -> super::AttemptDefinitionId {
    let definition = attempt
        .allocate_definition_builder(DefinitionIdentityPolicy::Disabled)
        .expect("definition builder");
    for word in parameters {
        attempt
            .push_definition_parameter(definition, word.token_word())
            .expect("parameter word");
    }
    attempt
        .finish_definition_parameters(definition)
        .expect("parameter boundary");
    for word in replacement {
        attempt
            .push_definition_replacement(definition, word.token_word())
            .expect("replacement word");
    }
    attempt.finish_definition(definition).expect("definition");
    definition
}

#[test]
fn mark_truncates_every_suffix_without_inspecting_values() {
    tex_state::with_universe(budget(), |universe| {
        let mut attempt = AttemptArena::default();
        let retained = attempt
            .allocate_token_list([word('a')])
            .expect("test fixture is valid");
        let mark = attempt.mark();
        let rejected = attempt
            .allocate_token_list([word('b'), word('c')])
            .expect("test fixture is valid");
        let rejected_glue = attempt
            .allocate_glue(glue(17))
            .expect("test fixture is valid");
        let rejected_name = attempt
            .allocate_name("discarded")
            .expect("test fixture is valid");

        attempt.truncate(mark).expect("test fixture is valid");

        assert_eq!(
            attempt
                .token_words(retained)
                .expect("test fixture is valid"),
            &[word('a')]
        );
        assert_eq!(
            attempt.token_words(rejected),
            Err(AttemptError::InvalidCoordinate)
        );
        assert_eq!(
            attempt.glue(rejected_glue),
            Err(AttemptError::InvalidCoordinate)
        );
        assert_eq!(
            attempt.name(rejected_name),
            Err(AttemptError::InvalidCoordinate)
        );
        let mut destination = TestPromotionDestination::new(&[retained], &[], &[], &[]);
        attempt
            .promote_into(universe, &mut destination)
            .expect("test fixture is valid");
        assert_eq!(
            universe
                .command_context()
                .expect("test fixture is valid")
                .token_list(destination.token_list(0).clone())
                .iter()
                .collect::<Vec<_>>(),
            &[word('a').token_word()]
        );
    })
    .expect("test fixture is valid");
}

#[test]
fn foreign_marks_and_offsets_are_rejected() {
    tex_state::with_universe(budget(), |_universe| {
        let first = AttemptArena::<()>::default();
        let mark = first.mark();
        let mut second = AttemptArena::<()>::default();
        assert_eq!(second.truncate(mark), Err(AttemptError::ForeignAttempt));
    })
    .expect("test fixture is valid");
}

#[test]
fn promotion_follows_only_declared_roots_and_owned_definition_builders() {
    tex_state::with_universe(budget(), |universe| {
        let mut attempt = AttemptArena::default();
        let replacement = attempt
            .allocate_token_list([word('x')])
            .expect("test fixture is valid");
        let unrelated = attempt
            .allocate_token_list([word('z')])
            .expect("test fixture is valid");
        let definition = definition(&mut attempt, &[word('#')], &[word('x')]);
        let promoted_glue = attempt
            .allocate_glue(glue(42))
            .expect("test fixture is valid");
        let unrelated_glue = attempt
            .allocate_glue(glue(99))
            .expect("test fixture is valid");

        let mut destination =
            TestPromotionDestination::new(&[replacement], &[promoted_glue], &[definition], &[]);
        attempt
            .promote_into(universe, &mut destination)
            .expect("test fixture is valid");

        assert_eq!(
            universe
                .command_context()
                .expect("test fixture is valid")
                .glue(destination.glue(0)),
            glue(42)
        );
        assert_eq!(
            universe
                .command_context()
                .expect("test fixture is valid")
                .definition(destination.definition(0).clone())
                .replacement_text(),
            &[word('x').token_word()]
        );
        assert_eq!(
            universe
                .retire()
                .expect("test fixture is valid")
                .token_list_rows(),
            1,
            "the unrelated list and builder words were not independently promoted"
        );
        let _ = (unrelated, unrelated_glue);
    })
    .expect("test fixture is valid");
}

#[test]
fn generic_cold_promotion_preserves_checked_definition_content() {
    tex_state::with_universe(budget(), |universe| {
        let mut attempt = AttemptArena::default();
        let definition = definition(&mut attempt, &[], &[word('x'); 32]);
        let mut destination = TestPromotionDestination::new(&[], &[], &[definition], &[]);
        attempt
            .promote_into(universe, &mut destination)
            .expect("definition promotion");
        let context = universe.command_context().expect("admission");
        let promoted_view = context.definition(destination.definition(0).clone());
        assert_eq!(promoted_view.replacement_text().len(), 32);
        assert!(attempt.definition_builder(definition).is_err());
        let recycled = attempt
            .allocate_definition_builder(DefinitionIdentityPolicy::Disabled)
            .expect("recycled builder");
        let recycled = attempt
            .definition_builder(recycled)
            .expect("recycled builder");
        assert!(recycled.words().is_empty());
        assert!(recycled.capacity() >= 32);
    })
    .expect("test fixture is valid");
}

#[cfg(feature = "profiling")]
#[test]
fn warmed_generic_single_definition_promotion_uses_no_semantic_apply_allocations() {
    tex_state::with_universe(budget(), |universe| {
        let mut attempt = AttemptArena::default();
        let warm = definition(&mut attempt, &[], &[word('x')]);
        let mut warm_destination = TestPromotionDestination::new(&[], &[], &[warm], &[]);
        attempt
            .promote_into(universe, &mut warm_destination)
            .expect("warm generic promotion");

        let definition = definition(&mut attempt, &[], &[word('x')]);
        let storage = attempt
            .definition_builder(definition)
            .expect("measured builder")
            .words()
            .as_ptr();
        let owner = tex_state::measurement::HotCoreAllocationOwner::SemanticApply;
        let before = tex_state::measurement::hot_core_thread_allocation_measurement(owner);
        let mut destination = TestPromotionDestination::new(&[], &[], &[definition], &[]);
        {
            let _scope = tex_state::measurement::hot_core_allocation_scope(owner);
            attempt
                .promote_into(universe, &mut destination)
                .expect("measured generic promotion");
        }
        let after = tex_state::measurement::hot_core_thread_allocation_measurement(owner);
        assert_eq!(after.calls - before.calls, 0);
        assert_eq!(after.requested_bytes - before.requested_bytes, 0);

        let context = universe.command_context().expect("definition context");
        let promoted = context.definition(destination.definition(0).clone());
        assert_eq!(promoted.replacement_text(), [word('x').token_word()]);
        assert_eq!(
            promoted.replacement_text().as_ptr(),
            storage,
            "generic promotion moves the builder words without copying them"
        );
    })
    .expect("test fixture is valid");
}

#[test]
fn generic_policy_mismatch_restores_the_builder_and_publishes_nothing() {
    tex_state::with_universe(budget(), |universe| {
        let mut attempt = AttemptArena::default();
        let token = attempt
            .allocate_token_list([word('t')])
            .expect("token root");
        let glue = attempt.allocate_glue(glue(7)).expect("glue root");
        let definition = attempt
            .allocate_definition_builder(DefinitionIdentityPolicy::Enabled)
            .expect("definition builder");
        attempt
            .finish_definition_parameters(definition)
            .expect("parameter boundary");
        attempt
            .push_definition_replacement(definition, word('x').token_word())
            .expect("replacement word");
        attempt.finish_definition(definition).expect("definition");
        let storage = attempt
            .definition_builder(definition)
            .expect("live builder")
            .words()
            .as_ptr();

        let mut destination = TestPromotionDestination::new(&[token], &[glue], &[definition], &[]);
        assert!(matches!(
            attempt.promote_into(universe, &mut destination),
            Err(AttemptError::Promotion(
                tex_state::PromotionError::IdentityPolicyMismatch
            ))
        ));
        assert_eq!(
            attempt
                .definition_builder(definition)
                .expect("rejected builder is restored")
                .words()
                .as_ptr(),
            storage
        );
        assert!(matches!(
            destination.token_lists[0],
            TestRoot::Attempt(root) if root == token
        ));
        assert!(matches!(
            destination.glue[0],
            TestRoot::Attempt(root) if root == glue
        ));
        assert!(matches!(
            destination.definitions[0],
            TestRoot::Attempt(root) if root == definition
        ));
        let retired = universe.retire().expect("retirement");
        assert_eq!(retired.definition_rows(), 0);
        assert_eq!(retired.token_list_rows(), 0);
        assert_eq!(retired.glue_rows(), 0);
    })
    .expect("test fixture is valid");
}

#[test]
fn promotion_copies_only_declared_provenance_roots() {
    tex_state::with_universe(budget(), |universe| {
        let mut attempt = AttemptArena::default();
        let retained = attempt
            .allocate_provenance(tex_state::provenance::OriginRecord::UnknownBootstrap)
            .expect("test fixture is valid");
        let discarded = attempt
            .allocate_provenance(tex_state::provenance::OriginRecord::UnknownBootstrap)
            .expect("test fixture is valid");

        let mut destination = TestPromotionDestination::new(&[], &[], &[], &[retained]);
        attempt
            .promote_into(universe, &mut destination)
            .expect("test fixture is valid");

        assert_eq!(
            universe
                .command_context()
                .expect("test fixture is valid")
                .provenance(destination.provenance(0)),
            tex_state::provenance::OriginRecord::UnknownBootstrap
        );
        let _ = discarded;
    })
    .expect("test fixture is valid");
}

#[test]
fn nested_builders_keep_outer_and_inner_scratch_disjoint() {
    tex_state::with_universe(budget(), |_universe| {
        let mut attempt = AttemptArena::<()>::default();
        let outer = attempt.begin_token_list().expect("test fixture is valid");
        attempt
            .push_token(outer, word('a'))
            .expect("test fixture is valid");
        let inner = attempt.begin_token_list().expect("test fixture is valid");
        attempt
            .push_token(inner, word('x'))
            .expect("test fixture is valid");
        let inner = attempt
            .finish_token_list(inner)
            .expect("test fixture is valid");
        attempt
            .push_token(outer, word('b'))
            .expect("test fixture is valid");
        let outer = attempt
            .finish_token_list(outer)
            .expect("test fixture is valid");

        assert_eq!(
            attempt.token_words(inner).expect("test fixture is valid"),
            &[word('x')]
        );
        assert_eq!(
            attempt.token_words(outer).expect("test fixture is valid"),
            &[word('a'), word('b')]
        );
    })
    .expect("test fixture is valid");
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
            .push_buffer_token(buffer, word('a'))
            .expect("test fixture is valid");
        attempt
            .push_buffer_token(buffer, word('b'))
            .expect("test fixture is valid");
        assert_eq!(
            attempt.token_buffer(buffer).expect("test fixture is valid"),
            &[word('a'), word('b')]
        );
        let retained_chunks = attempt.token_lane.retained_chunks();
        let frozen = attempt
            .finish_token_buffer(buffer)
            .expect("test fixture is valid");
        assert_eq!(
            attempt.token_words(frozen).expect("test fixture is valid"),
            &[word('a'), word('b')]
        );
        let AttemptTokenStorage::Buffer(frozen_buffer) = attempt.token_lists[frozen.index()].value
        else {
            panic!("finished scanner result addresses its parent sink")
        };
        assert_eq!(frozen_buffer, buffer);
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
            attempt.token_buffer(buffer),
            Err(AttemptError::InvalidCoordinate)
        );
        assert!(
            attempt
                .token_buffer(recycled)
                .expect("recycled sink")
                .is_empty()
        );
        assert_eq!(
            attempt.token_words(frozen),
            Err(AttemptError::InvalidCoordinate)
        );
        attempt.truncate(mark).expect("recycled buffer retires");
        assert_eq!(
            attempt.token_buffer(recycled),
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
            .push_buffer_token(outer, word('a'))
            .expect("outer scanner word");
    }

    let child_opening = attempt.mark();
    let child = attempt
        .allocate_token_buffer()
        .expect("child scanner destination");
    for _ in 0..65 {
        attempt
            .push_buffer_token(child, word('x'))
            .expect("child scanner word");
    }
    attempt
        .truncate(child_opening)
        .expect("child scanner suffix truncates");

    attempt
        .push_buffer_token(outer, word('b'))
        .expect("outer destination continues in place");
    let outer = attempt
        .finish_token_buffer(outer)
        .expect("outer scanner result");
    let words = attempt.token_words(outer).expect("outer scanner words");
    assert_eq!(words.len(), 66);
    assert!(words.iter().take(65).all(|entry| *entry == word('a')));
    assert_eq!(words.get(65).copied(), Some(word('b')));
    assert_eq!(
        attempt.token_buffer(child),
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
            .push_buffer_token(parent, word('p'))
            .expect("parent prefix");
        let child = attempt
            .allocate_token_buffer()
            .expect("child scanner destination");
        for _ in 0..token_count {
            attempt
                .push_buffer_token(child, word('c'))
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
                .consume_token_list_into_buffer(child, parent)
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
            attempt.token_words(child),
            Err(AttemptError::InvalidCoordinate)
        );
        let parent = attempt
            .finish_token_buffer(parent)
            .expect("completed parent list");
        let words = attempt
            .token_words(parent)
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
                .push_buffer_token(buffer, word('x'))
                .expect("scanner word");
        }
        let before_finish = attempt.token_lane.counters();
        let result = attempt.finish_token_buffer(buffer).expect("scanner result");
        assert_eq!(
            attempt.token_lane.counters(),
            before_finish,
            "finalization only publishes the existing sink coordinate"
        );
        let words = attempt.token_words(result).expect("scanner words");
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
fn retired_definition_builder_reuses_its_word_allocation() {
    let mut attempt = AttemptArena::<()>::default();
    let mark = attempt.mark();
    let definition = attempt
        .allocate_definition_builder(DefinitionIdentityPolicy::Disabled)
        .expect("definition builder");
    attempt
        .finish_definition_parameters(definition)
        .expect("parameter boundary");
    for _ in 0..32 {
        attempt
            .push_definition_replacement(definition, word('x').token_word())
            .expect("replacement word");
    }
    attempt
        .finish_definition(definition)
        .expect("complete definition");
    let storage = attempt
        .definition_builder(definition)
        .expect("live builder")
        .words()
        .as_ptr();

    attempt.truncate(mark).expect("retire definition");
    let recycled = attempt
        .allocate_definition_builder(DefinitionIdentityPolicy::Disabled)
        .expect("recycled builder");
    assert_eq!(
        attempt
            .definition_builder(recycled)
            .expect("live recycled builder")
            .words()
            .as_ptr(),
        storage
    );
    assert_eq!(
        attempt
            .definition_builder(recycled)
            .expect("live recycled builder")
            .capacity(),
        32
    );
}

#[test]
fn definition_builder_ids_reject_foreign_stale_and_double_finish_without_mutation() {
    let mut first = AttemptArena::<()>::default();
    let definition = first
        .allocate_definition_builder(DefinitionIdentityPolicy::Disabled)
        .expect("definition builder");
    let mut foreign = AttemptArena::<()>::default();
    let foreign_mark = foreign.mark();
    assert_eq!(
        foreign.push_definition_parameter(definition, word('x').token_word()),
        Err(AttemptError::ForeignAttempt)
    );
    assert_eq!(foreign.mark(), foreign_mark);

    first
        .finish_definition_parameters(definition)
        .expect("parameter boundary");
    first
        .push_definition_replacement(definition, word('x').token_word())
        .expect("replacement word");
    first.finish_definition(definition).expect("first finish");
    let words = first
        .definition_builder(definition)
        .expect("sealed builder")
        .words()
        .to_vec();
    let capacity = first
        .definition_builder(definition)
        .expect("sealed builder")
        .capacity();
    assert_eq!(
        first.finish_definition(definition),
        Err(AttemptError::Definition(DefinitionBuildError::InvalidPhase))
    );
    let unchanged = first
        .definition_builder(definition)
        .expect("failed double finish retains builder");
    assert_eq!(unchanged.words(), words);
    assert_eq!(unchanged.capacity(), capacity);

    let stale_mark = first.mark();
    let stale = first
        .allocate_definition_builder(DefinitionIdentityPolicy::Disabled)
        .expect("stale candidate");
    first.truncate(stale_mark).expect("retire candidate");
    let after_retirement = first.mark();
    assert_eq!(
        first.push_definition_parameter(stale, word('y').token_word()),
        Err(AttemptError::InvalidCoordinate)
    );
    assert_eq!(first.mark(), after_retirement);
}

#[cfg(feature = "profiling")]
#[test]
fn warmed_definition_builder_attempts_allocate_zero_heap() {
    let mut attempt = AttemptArena::<()>::default();
    let run = |attempt: &mut AttemptArena<()>| {
        let mark = attempt.mark();
        let definition = attempt
            .allocate_definition_builder(DefinitionIdentityPolicy::Disabled)
            .expect("definition builder");
        attempt
            .finish_definition_parameters(definition)
            .expect("parameter boundary");
        for _ in 0..32 {
            attempt
                .push_definition_replacement(definition, word('x').token_word())
                .expect("replacement word");
        }
        attempt
            .finish_definition(definition)
            .expect("complete definition");
        attempt.truncate(mark).expect("retire definition");
    };
    for _ in 0..64 {
        run(&mut attempt);
    }
    let owner = tex_state::measurement::HotCoreAllocationOwner::AttemptScratch;
    let before = tex_state::measurement::hot_core_thread_allocation_measurement(owner);
    for _ in 0..8_192 {
        run(&mut attempt);
    }
    let after = tex_state::measurement::hot_core_thread_allocation_measurement(owner);
    assert_eq!(after.calls - before.calls, 0);
    assert_eq!(after.requested_bytes - before.requested_bytes, 0);
}

#[test]
fn attempt_local_provenance_stays_aligned_through_nested_builders() {
    tex_state::with_universe(budget(), |_universe| {
        let mut attempt = AttemptArena::<()>::default();
        let origin = attempt
            .allocate_provenance(tex_state::provenance::OriginRecord::UnknownBootstrap)
            .expect("test fixture is valid");
        let outer = attempt.begin_token_list().expect("test fixture is valid");
        attempt
            .push_token_with_local_origin(outer, word('a').token_word(), origin)
            .expect("test fixture is valid");
        let inner = attempt.begin_token_list().expect("test fixture is valid");
        attempt
            .push_token(inner, word('x'))
            .expect("test fixture is valid");
        let inner = attempt
            .finish_token_list(inner)
            .expect("test fixture is valid");
        let outer = attempt
            .finish_token_list(outer)
            .expect("test fixture is valid");

        assert_eq!(
            attempt
                .token_origin(outer, 0)
                .expect("test fixture is valid"),
            super::AttemptOrigin::Local(origin)
        );
        assert_eq!(
            attempt
                .token_origin(inner, 0)
                .expect("test fixture is valid"),
            super::AttemptOrigin::Admitted(OriginId::UNKNOWN)
        );
    })
    .expect("test fixture is valid");
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
            attempt.token_words(stale),
            Err(AttemptError::InvalidCoordinate)
        );
        assert_eq!(
            attempt
                .token_words(replacement)
                .expect("test fixture is valid"),
            &[word('b')]
        );
    })
    .expect("test fixture is valid");
}

#[test]
fn pending_attempt_owns_generation_and_resumes_without_a_borrow() {
    tex_state::with_universe(budget(), |universe| {
        let generation = universe.generation_owner().expect("test fixture is valid");
        let pending = PendingCommandAttempt::new(
            CommandAttempt::default(),
            generation,
            AttemptResumePoint {
                command: 7,
                scanner: 11,
                expansion: 13,
                subordinate: 17,
            },
            "font request",
        );
        let allocated_while_pinned = universe
            .allocate_token_list(&[])
            .expect("a coarse owner pins retirement, not append-only allocation");
        assert!(
            universe
                .command_context()
                .expect("context")
                .token_list(allocated_while_pinned)
                .is_empty()
        );
        assert_eq!(
            universe.retire(),
            Err(tex_state::UniverseError::State(
                tex_state::StateError::GenerationInUse
            ))
        );

        let (attempt, _opening, resume, request) = pending
            .resume(universe)
            .ok()
            .expect("test fixture is valid");
        assert!(!attempt.is_empty());
        assert_eq!(resume.command, 7);
        assert_eq!(request, "font request");
        assert!(attempt.arena().mark().traced_words == 0);
        universe
            .allocate_token_list(&[])
            .expect("test fixture is valid");
    })
    .expect("test fixture is valid");
}

#[test]
fn pending_owner_rejects_retirement_without_partially_retiring_universe() {
    tex_state::with_universe(budget(), |universe| {
        let owner = universe.generation_owner().expect("test fixture is valid");
        assert_eq!(
            universe.retire(),
            Err(tex_state::UniverseError::State(
                tex_state::StateError::GenerationInUse
            ))
        );
        assert!(!universe.is_retired());
        universe
            .intern("still-live")
            .expect("test fixture is valid");
        drop(owner);
        universe.retire().expect("test fixture is valid");
    })
    .expect("test fixture is valid");
}

#[test]
fn owned_scopes_close_exact_lifo_and_reject_stale_coordinates() {
    let mut attempt = AttemptArena::<()>::default();
    let retained = attempt
        .allocate_token_list([word('p')])
        .expect("parent value");
    let parent = attempt.begin_owned_scope().expect("parent scope");
    let parent_value = attempt
        .allocate_token_list([word('a')])
        .expect("parent-scope value");
    let child = attempt.begin_owned_scope().expect("child scope");
    let child_value = attempt
        .allocate_token_list([word('b')])
        .expect("child-scope value");

    assert_eq!(
        attempt.validate_top_owner(&parent),
        Err(AttemptError::InvalidCoordinate)
    );
    assert_eq!(
        attempt.token_words(child_value).expect("child words"),
        &[word('b')]
    );
    attempt
        .close_owned_scope(child)
        .expect("child closes first");
    assert_eq!(
        attempt.token_words(child_value),
        Err(AttemptError::InvalidCoordinate)
    );
    assert_eq!(
        attempt.token_words(parent_value).expect("parent words"),
        &[word('a')]
    );
    attempt
        .close_owned_scope(parent)
        .expect("parent closes second");
    assert_eq!(
        attempt.token_words(retained).expect("retained words"),
        &[word('p')]
    );
}

#[test]
fn lexical_scope_truncates_its_branded_child_id() {
    let mut attempt = AttemptArena::<()>::default();
    attempt
        .with_child_scope(|scope| {
            let child = scope
                .allocate_token_list([word('x')])
                .expect("child allocation");
            assert_eq!(
                scope.token_words(&child).expect("child words"),
                &[word('x')]
            );
        })
        .expect("lexical scope");
    assert!(attempt.mark().is_empty());
}

#[test]
fn four_hundred_thousand_scope_handoffs_keep_arena_metadata_constant() {
    let mut attempt = AttemptArena::<()>::default();
    let operation = attempt.begin_owned_scope().expect("operation scope");
    let mut scanner = attempt.begin_owned_scope().expect("scanner scope");
    let output = attempt
        .allocate_token_buffer()
        .expect("parent-owned scanner sink");

    for _ in 0..400_000 {
        let child = attempt.begin_owned_scope().expect("macro child");
        let retired = attempt
            .allocate_token_list([word('x')])
            .expect("child scratch");
        attempt
            .close_owned_scope(child)
            .expect("top child retires immediately");
        assert_eq!(attempt.top_scope, scanner.serial);
        assert!(attempt.token_buffer(output).is_ok());
        assert_eq!(
            attempt.token_words(retired),
            Err(AttemptError::InvalidCoordinate)
        );
    }

    attempt
        .handoff_owned_parent(operation, &mut scanner)
        .expect("scanner consumes its operation parent");
    assert!(attempt.token_buffer(output).is_ok());
    attempt
        .close_owned_scope(scanner)
        .expect("operation consumes scanner then itself");
    assert!(attempt.mark().is_empty());
}

#[test]
fn deferred_scanner_result_survives_a_younger_immediate_retirement() {
    let mut attempt = AttemptArena::<()>::default();
    let operation = attempt.begin_owned_scope().expect("operation scope");
    let mut scanner = attempt.begin_owned_scope().expect("scanner scope");
    let result = attempt
        .allocate_token_list([word('s')])
        .expect("scanner result");
    let macro_child = attempt.begin_owned_scope().expect("macro child");
    let scratch = attempt
        .allocate_token_list([word('x')])
        .expect("macro scratch");
    attempt
        .close_owned_scope(macro_child)
        .expect("younger macro retires immediately");

    assert_eq!(
        attempt.token_words(result).expect("result words"),
        &[word('s')]
    );
    assert_eq!(
        attempt.token_words(scratch),
        Err(AttemptError::InvalidCoordinate)
    );
    attempt
        .handoff_owned_parent(operation, &mut scanner)
        .expect("scanner consumes its operation parent");
    attempt
        .close_owned_scope(scanner)
        .expect("commit releases scanner result and operation");
    assert!(attempt.mark().is_empty());
}

#[test]
fn successful_handoff_does_not_consult_an_obsolete_child_opening_mark() {
    let mut attempt = AttemptArena::<()>::default();
    let operation = attempt.begin_owned_scope().expect("operation scope");
    let operation_opening = operation.opening;
    attempt
        .allocate_token_list([word('p')])
        .expect("operation-local promoted prefix");
    let mut scanner = attempt.begin_owned_scope().expect("scanner scope");

    // Successful root publication may reclaim rows which preceded the child
    // scope. The child opening is then intentionally stale, while its merged
    // close-through operation mark remains the exact surviving boundary.
    attempt
        .truncate(operation_opening)
        .expect("published prefix reclaims without closing the live scope");
    assert_eq!(
        attempt.validate_mark(scanner.opening),
        Err(AttemptError::InvalidCoordinate)
    );
    attempt
        .handoff_owned_parent(operation, &mut scanner)
        .expect("handoff uses typed ownership, not obsolete opening lengths");
    attempt
        .close_owned_scope(scanner)
        .expect("successful close uses the merged close-through mark");
    assert_eq!(attempt.top_scope, AttemptScopeSerial::ROOT);
    assert!(attempt.mark().is_empty());
}

#[test]
fn preallocated_scanner_sink_survives_a_younger_operation_rollback() {
    let mut attempt = AttemptArena::<()>::default();
    let opening_operation = attempt.begin_owned_scope().expect("opening operation");
    let mut scanner = attempt.begin_owned_scope().expect("scanner scope");
    let output = attempt
        .allocate_token_buffer()
        .expect("scanner reserves its parent sink");
    attempt
        .handoff_owned_parent(opening_operation, &mut scanner)
        .expect("scanner keeps the opening operation below it");

    let rejected_retry = attempt.begin_owned_scope().expect("rejected retry");
    let rejected = attempt
        .allocate_token_list([word('x')])
        .expect("retry-local scratch");
    attempt
        .close_owned_scope(rejected_retry)
        .expect("retry suffix rolls back");
    assert!(
        attempt
            .token_buffer(output)
            .expect("output sink")
            .is_empty()
    );
    assert_eq!(
        attempt.token_words(rejected),
        Err(AttemptError::InvalidCoordinate)
    );

    let mut completed_retry = attempt.begin_owned_scope().expect("completed retry");
    attempt
        .push_buffer_token(output, word('r'))
        .expect("retry writes through the scanner-owned sink");
    let result = attempt
        .finish_token_buffer(output)
        .expect("parent sink finalizes after retry");
    attempt
        .handoff_owned_parent(scanner, &mut completed_retry)
        .expect("result survives through retry commit");
    assert_eq!(
        attempt.token_words(result).expect("result words"),
        &[word('r')]
    );
    attempt
        .close_owned_scope(completed_retry)
        .expect("completed retry closes the whole retired suffix");
    assert!(attempt.mark().is_empty());
}

#[test]
fn foreign_deferred_child_is_rejected_without_consuming_the_live_operation() {
    let mut attempt = CommandAttempt::<()>::default();
    let operation = attempt.begin_operation(0).expect("operation scope");
    let mut foreign = CommandAttempt::<()>::default();
    let _foreign_operation = foreign.begin_operation(0).expect("foreign operation");
    let foreign_child = foreign.begin_child_scope().expect("foreign child");

    assert_eq!(
        attempt.defer_child_to_operation(foreign_child),
        Err(AttemptError::ForeignAttempt)
    );
    assert!(attempt.validate_operation(operation).is_ok());
    assert_eq!(attempt.rollback_operation(operation), Ok(()));
    assert!(attempt.is_empty());
}

#[test]
fn rollback_truncates_a_child_that_inherited_the_operation_owner() {
    let mut attempt = CommandAttempt::<()>::default();
    let operation = attempt.begin_operation(0).expect("operation scope");
    let child = attempt.begin_child_scope().expect("scanner child");
    let rejected = attempt
        .arena_mut()
        .allocate_token_list([word('x')])
        .expect("child scratch");
    assert_eq!(
        attempt.defer_child_to_operation(child),
        Ok(()),
        "the child directly inherits the operation"
    );

    assert_eq!(attempt.rollback_operation(operation), Ok(()));
    assert_eq!(
        attempt.arena().token_words(rejected),
        Err(AttemptError::InvalidCoordinate)
    );
    assert!(attempt.is_empty());
}
