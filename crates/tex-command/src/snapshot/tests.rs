use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use crate::CommandState;
use crate::conditionals::{ConditionFrame, ConditionalKind, IfLimit};
use crate::input::{
    BackedUpToken, BackupTreatment, InputLevel, ReplayTrace, RetirementBehavior, TokenBehavior,
    TokenPayload,
};
use crate::macro_call::{MacroActivationId, MacroArgumentRange};
use crate::processor::{
    AbsorbingContext, ActiveCellDelivery, AlignmentCellTemplates, AlignmentId, AlignmentIdentity,
    AlignmentScanContext, ArgumentBuilderId, ConditionId, DefinitionContext, MatchingContext,
    ScannerStatus, ScannerWarning, SkippingContext, SuspendedAlignment, TokenBuilderId,
};
use crate::profile::{CommandProfile, CommandProfileBoundary, CommandProfileFingerprint};
use crate::state::LiveTokenBuilder;
use crate::{RegisteredSourceKind, SourceRegistration};
use tex_state::ids::TokenListId;
use tex_state::input::TracedTokenList;
use tex_state::macro_store::{MacroDefinitionProvenance, MacroMeaning};
use tex_state::meaning::MeaningFlags;
use tex_state::provenance::InsertedOriginKind;
use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};

use super::{CommandSummary, CommandSummaryError};

fn semantic_hash<T: Hash>(value: &T) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

fn inline_word(ch: char) -> TracedTokenWord {
    TracedTokenWord::pack(
        Token::Char {
            ch,
            cat: Catcode::Other,
        },
        OriginId::UNKNOWN,
    )
}

#[test]
fn durable_continuation_roundtrip_preserves_inline_payloads() {
    let mut state = CommandState::default();
    state.push_token_level(
        TokenPayload::transient([inline_word('t')]),
        TokenBehavior::Recovery,
        RetirementBehavior::Pop,
        ReplayTrace::Inserted,
    );
    state.push_token_level(
        TokenPayload::backed_up([BackedUpToken {
            spelling: inline_word('b'),
            source_provenance: None,
        }]),
        TokenBehavior::BackedUp(BackupTreatment::Ordinary),
        RetirementBehavior::Pop,
        ReplayTrace::BackedUp,
    );
    let summary = state.publish_summary().expect("inline state is quiescent");
    let universe = tex_state::Universe::new();
    let owned = crate::OwnedCommandContinuation::detach(&summary, &universe);
    let mut restored_universe = tex_state::Universe::new();
    let restored = owned
        .materialize(&mut restored_universe)
        .expect("valid detached continuation");

    let mut payloads = restored
        .input
        .levels
        .iter()
        .filter_map(|level| match level {
            crate::input::InputLevel::Tokens(cursor) => Some(&cursor.payload),
            crate::input::InputLevel::Source(_) => None,
        });
    assert!(matches!(
        payloads.next(),
        Some(TokenPayload::Packed(chunk)) if chunk.word(0) == Some(inline_word('t'))
    ));
    assert!(matches!(
        payloads.next(),
        Some(TokenPayload::Packed(chunk)) if chunk.word(0) == Some(inline_word('b'))
            && chunk.is_backed_up()
    ));
}

#[test]
fn durable_continuation_materializes_canonical_stored_content_into_new_roots() {
    let mut universe = tex_state::Universe::new();
    let symbol = universe.intern("detached-root").symbol();
    let source_root = universe.intern_token_list_ref(&[Token::Cs(symbol)]);
    let mut state = CommandState::default();
    state.push_token_level(
        TokenPayload::stored(
            universe.tokens(source_root.id()).tokens(),
            tex_state::provenance::OriginListRef::empty(),
        ),
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::Inserted,
    );
    let summary = state.publish_summary().expect("stored state is quiescent");
    let owned = crate::OwnedCommandContinuation::detach(&summary, &universe);

    let mut restored_universe = tex_state::Universe::new();
    let _different_first_coordinate = restored_universe.intern_token_list_ref(&[Token::Char {
        ch: 'x',
        cat: Catcode::Other,
    }]);
    let restored = owned
        .materialize(&mut restored_universe)
        .expect("valid detached continuation");
    let crate::input::InputLevel::Tokens(cursor) =
        restored.input.levels.last().expect("materialized level")
    else {
        panic!("materialized the wrong level kind");
    };
    let TokenPayload::Packed(tokens) = &cursor.payload else {
        panic!("materialized the wrong payload kind");
    };
    let Some(Token::Cs(restored_symbol)) = tokens.word(0).map(|word| word.semantic_token()) else {
        panic!("stored control sequence did not materialize");
    };
    assert_eq!(restored_universe.resolve(restored_symbol), "detached-root");
}

#[test]
fn durable_continuation_roundtrips_source_macro_and_frame_recipes() {
    let mut world = tex_state::World::memory();
    world
        .set_memory_file("/job/main.tex", b"abc\n".to_vec())
        .expect("World source backing installs");
    let mut universe = tex_state::Universe::with_world(world);
    let content = universe
        .world_mut()
        .read_file("/job/main.tex")
        .expect("World source backing reads");
    let mut state = CommandState::default();
    let source_id = state
        .register_source(SourceRegistration::world(content).with_name("/job/main.tex"))
        .expect("source recipe is valid");
    state
        .open_registered_source(source_id)
        .expect("registered source opens");
    let InputLevel::Source(source_level) = state.input.levels.last().expect("source level") else {
        panic!("wrong input level");
    };
    universe
        .register_source(source_id, source_level.cursor.backing.source_descriptor())
        .expect("aggregate source registration");

    let invocation = universe.source_range_origin_ref(source_id, 0, 1);
    let definition = universe.source_range_origin_ref(source_id, 1, 2);
    let parameter_origins = universe.allocate_origin_list_ref(&[]);
    let replacement_origins = universe.allocate_origin_list_ref(std::slice::from_ref(&definition));
    let parameter_tokens = universe.intern_token_list_ref(&[]);
    let macro_name = universe.intern("detached-macro").symbol();
    let replacement_tokens = universe.intern_token_list_ref(&[Token::Cs(macro_name)]);
    let macro_root = universe.intern_macro_with_provenance(
        MacroMeaning::new(
            MeaningFlags::from_bits(0),
            parameter_tokens.id(),
            replacement_tokens.id(),
        ),
        MacroDefinitionProvenance::new(definition.clone(), parameter_origins, replacement_origins),
    );
    let frame = universe.macro_invocation_frame(
        macro_root.id(),
        invocation.clone(),
        definition.clone(),
        tex_state::provenance::OriginRef::unknown(),
    );
    let inserted = universe.inserted_origin_ref(
        InsertedOriginKind::AfterAssignment,
        Token::Cs(macro_name),
        frame.as_origin().clone(),
    );
    state.parameters.admit_macro(
        macro_root.id(),
        universe.macro_definition(macro_root.id()).meaning(),
    );
    let arguments = state.parameters.store_arguments(
        tex_state::token::RootedTracedTokenBuffer::new([
            tex_state::token::RootedTracedTokenWord::new(Token::Cs(macro_name), inserted.clone()),
        ]),
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
    state.parameters.restore_activation(
        MacroActivationId(7),
        macro_name,
        macro_root.id(),
        arguments,
        frame.id(),
    );
    state.parameters.next_activation_identity = 11;
    let stored_origins = universe.allocate_origin_list_ref(&[inserted]);
    state.push_token_level(
        TokenPayload::stored(
            universe.tokens(replacement_tokens.id()).tokens(),
            stored_origins,
        ),
        TokenBehavior::MacroBody(MacroActivationId(7)),
        RetirementBehavior::Pop,
        ReplayTrace::MacroReplacement,
    );
    state.conditions.frames.push(ConditionFrame {
        identity: ConditionId(13),
        kind: ConditionalKind::IfTrue,
        limit: IfLimit::Fi,
        source_line: 1,
        inverted: false,
    });
    state.expansion.cumulative_expansions = 17;
    let summary = state.publish_summary().expect("state is quiescent");
    let source_invocations = universe.macro_invocation_provenance_stats();
    assert_eq!(source_invocations.invocations(), 1);
    let owned = crate::OwnedCommandContinuation::detach(&summary, &universe);

    let mut destination = tex_state::Universe::new();
    let foreign_symbol = destination.intern("foreign").symbol();
    let _foreign_tokens = destination.intern_token_list_ref(&[Token::Cs(foreign_symbol)]);
    let restored = owned
        .materialize(&mut destination)
        .expect("complete recipes materialize");

    assert_eq!(restored.conditions, summary.conditions);
    assert_eq!(restored.align_state, summary.align_state);
    assert_eq!(restored.expansion, summary.expansion);
    assert_eq!(restored.parameters.next_activation_identity, 11);
    let restored_activation = restored.parameters.activations.last().expect("activation");
    assert_eq!(
        destination
            .macro_invocation_provenance_stats()
            .invocations(),
        source_invocations.invocations(),
        "detached active frames must preserve the exact logical invocation count"
    );
    assert_ne!(
        restored_activation.definition,
        summary.parameters.activations[0].definition
    );
    assert_eq!(
        destination.resolve(restored_activation.name),
        "detached-macro"
    );
    let restored_owner = restored
        .parameters
        .macro_owner(restored_activation.definition);
    let restored_macro = destination.macro_definition(restored_owner);
    assert_eq!(
        destination
            .tokens(restored_macro.replacement_text())
            .tokens(),
        [Token::Cs(restored_activation.name)]
    );
    let tex_state::provenance::OriginRecord::MacroInvocation(restored_frame) =
        destination.origin(restored_activation.invocation)
    else {
        panic!("activation did not materialize an expansion frame");
    };
    assert_eq!(
        restored_frame.definition_operand(),
        destination.macro_definition_observation_operand(restored_activation.definition) as u64,
        "frame definition operand must be destination-local"
    );
    let resolved = tex_state::ProvenanceResolver::new(&destination)
        .resolve_origin(restored_activation.invocation)
        .expect("frame resolves through its invocation recipe");
    assert_eq!(resolved.path, "/job/main.tex");
    assert_eq!((resolved.start, resolved.end), (0, 1));
    assert_eq!(
        restored
            .input
            .levels
            .iter()
            .find_map(|level| match level {
                InputLevel::Tokens(cursor) => match &cursor.payload {
                    TokenPayload::Packed(chunk) => chunk.word(0)?.token(),
                    _ => None,
                },
                InputLevel::Source(_) => None,
            })
            .expect("packed level"),
        Token::Cs(restored_activation.name)
    );

    let mut restored_state = CommandState::default();
    restored_state
        .restore_summary(restored)
        .expect("materialized summary restarts");
    assert_eq!(restored_state.parameters.activations.len(), 1);
    assert_eq!(restored_state.conditions.frames.len(), 1);
    assert_eq!(
        destination
            .macro_invocation_provenance_stats()
            .invocations(),
        source_invocations.invocations(),
        "installing the restored command must keep its archived frame coordinate live"
    );
}

#[test]
fn invalid_continuation_recipe_rejects_before_publishing_roots() {
    let mut universe = tex_state::Universe::new();
    let symbol = universe.intern("invalid-recipe").symbol();
    let tokens = universe.intern_token_list_ref(&[Token::Cs(symbol)]);
    let mut state = CommandState::default();
    state.push_token_level(
        TokenPayload::stored(
            universe.tokens(tokens.id()).tokens(),
            tex_state::provenance::OriginListRef::empty(),
        ),
        TokenBehavior::Ordinary,
        RetirementBehavior::Pop,
        ReplayTrace::Inserted,
    );
    let summary = state.publish_summary().expect("state is quiescent");
    let mut owned = crate::OwnedCommandContinuation::detach(&summary, &universe);
    owned.corrupt_first_token_recipe_for_test();

    let mut destination = tex_state::Universe::new();
    let before = destination.provenance_stats();
    assert!(matches!(
        owned.materialize(&mut destination),
        Err(crate::CommandContinuationError::InvalidRecipe(_))
    ));
    assert_eq!(destination.provenance_stats(), before);
}

#[test]
fn continuation_destination_conflict_is_atomic_and_retryable() {
    let mut state = CommandState::default();
    let source_id = state
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            b"source".to_vec(),
        ))
        .expect("source recipe");
    state
        .open_registered_source(source_id)
        .expect("registered source opens");
    let InputLevel::Source(source) = state.input.levels.last().expect("source level") else {
        panic!("wrong level kind");
    };
    let mut universe = tex_state::Universe::new();
    universe
        .register_source(source_id, source.cursor.backing.source_descriptor())
        .expect("source map registration");
    let summary = state.publish_summary().expect("source state is quiescent");
    let owned = crate::OwnedCommandContinuation::detach(&summary, &universe);

    let mut conflicting = tex_state::Universe::new();
    let foreign =
        tex_state::source_map::SourceDescriptor::generated(std::sync::Arc::from(&b"foreign"[..]));
    conflicting
        .register_source(source_id, foreign.clone())
        .expect("foreign registration");
    let before = conflicting.provenance_stats();
    assert_eq!(
        owned.materialize(&mut conflicting),
        Err(crate::CommandContinuationError::SourceMap(
            tex_state::source_map::SourceMapError::ConflictingRegistration
        ))
    );
    assert_eq!(conflicting.provenance_stats(), before);
    assert_eq!(
        conflicting.detached_source_descriptor(source_id),
        Some(foreign)
    );

    let mut busy = tex_state::Universe::new();
    busy.begin_private_revision();
    let before_busy = busy.provenance_stats();
    assert_eq!(
        owned.materialize(&mut busy),
        Err(crate::CommandContinuationError::DestinationBusy)
    );
    assert_eq!(busy.provenance_stats(), before_busy);

    let mut retry = tex_state::Universe::new();
    let restored = owned
        .materialize(&mut retry)
        .expect("same detached value retries in a compatible destination");
    assert_eq!(restored.root_source_anchor(), Some(0));
}

#[test]
fn continuation_keeps_rebound_cursor_bytes_separate_from_source_registration() {
    let original = b"source";
    let rebound = b"edited source";
    let mut state = CommandState::default();
    let source_id = state
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            original.to_vec(),
        ))
        .expect("source recipe");
    state
        .open_registered_source(source_id)
        .expect("registered source opens");
    let InputLevel::Source(source) = state.input.levels.last().expect("source level") else {
        panic!("wrong level kind");
    };
    let original_descriptor = source.cursor.backing.source_descriptor();
    let mut retained = tex_state::Universe::new();
    retained
        .register_source(source_id, original_descriptor.clone())
        .expect("stable coordinate registration");
    let mut summary = state.publish_summary().expect("source state is quiescent");
    assert!(summary.rebind_root_source(original, std::sync::Arc::from(&rebound[..])));
    let owned = crate::OwnedCommandContinuation::detach(&summary, &retained);

    let mut destination = tex_state::Universe::new();
    destination
        .register_source(source_id, original_descriptor.clone())
        .expect("destination keeps the coordinate registration");
    let before = destination.provenance_stats();
    let restored = owned
        .materialize(&mut destination)
        .expect("rebound cursor materializes over its retained registration");

    assert!(restored.root_source_matches(rebound));
    assert_eq!(destination.provenance_stats(), before);
    assert_eq!(
        destination.detached_source_descriptor(source_id),
        Some(original_descriptor)
    );
}

#[test]
fn root_anchor_rounds_past_the_complete_loaded_physical_line() {
    let mut state = CommandState::default();
    let source = state
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            b"first\nsecond".to_vec(),
        ))
        .expect("source registers");
    state
        .open_registered_source(source)
        .expect("registered source opens");
    state.load_next_source_line(13).expect("first line loads");
    state
        .next_source_character()
        .expect("one scalar is consumed");

    let summary = state.publish_summary().expect("source state is quiescent");
    assert_eq!(summary.root_source_anchor(), Some(b"first\n".len()));
}

fn templates() -> AlignmentCellTemplates {
    let universe = tex_state::Universe::new();
    AlignmentCellTemplates {
        u_template: Some(TracedTokenList::synthetic(
            universe.token_list_ref(TokenListId::EMPTY),
        )),
        v_template: TracedTokenList::synthetic(universe.token_list_ref(TokenListId::EMPTY)),
    }
}

fn populated_quiescent_state() -> CommandState {
    let mut state = CommandState::new(CommandProfile::unicode_extended(
        crate::CommandDialect::Pdftex14029,
    ));
    let source = state
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Vec::from(&b"first  \r\nsecond"[..]),
        ))
        .expect("valid Unicode source");
    state
        .open_registered_source(source)
        .expect("registered source opens without host access");
    state
        .load_next_source_line(13)
        .expect("first physical line");
    state.next_source_character().expect("first scalar");
    state.input.next_level_identity = 11;
    state.input.next_source_identity = 13;
    let arguments = state.parameters.store_arguments(
        tex_state::token::RootedTracedTokenBuffer::default(),
        [
            MacroArgumentRange::new(0, 0),
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
    state.parameters.restore_activation(
        MacroActivationId(17),
        tex_state::interner::Symbol::testing_new(23),
        tex_state::macro_store::MacroDefinitionRef::testing_new(19).id(),
        arguments,
        tex_state::token::OriginId::UNKNOWN,
    );
    state.conditions.frames.push(ConditionFrame {
        identity: ConditionId(23),
        kind: ConditionalKind::IfNum,
        limit: IfLimit::Else,
        source_line: 37,
        inverted: false,
    });
    state.conditions.next_identity = 41;
    state.alignment.align_state = 43;
    state.expansion.cumulative_expansions = 47;
    state.expansion.next_resource_resolution = 53;
    state.expansion.pending_diagnostics.push(59);
    state.expansion.observed_dependencies.push(61);
    state.expansion.semantic_barriers.push(67);
    state.transient.next_builder_identity = 71;
    state
}

#[test]
fn snapshot_roundtrip_preserves_nonquiescent_semantic_state() {
    let mut state = populated_quiescent_state();
    let status = ScannerStatus::Matching(MatchingContext {
        macro_name: tex_state::interner::Symbol::testing_new(73),
        builder: ArgumentBuilderId(79),
        warning: ScannerWarning(83),
    });
    let (expected, snapshot) = state.with_scanner_status(status, |state| {
        state.transient.builders.push(LiveTokenBuilder {
            identity: 79,
            tokens: tex_state::token::RootedTracedTokenBuffer::default(),
        });
        state.transient.rollback_roots.push(89);
        state.transient.active_expansion_depth = 2;
        state.alignment.active_cell = Some(ActiveCellDelivery {
            alignment: AlignmentIdentity::new(97),
            templates: templates(),
            u_template_installed: false,
            u_level: None,
            v_level: None,
            delimiter: None,
            omit: false,
            omit_previous_align_state: None,
        });
        (state.clone(), state.snapshot())
    });

    state = CommandState::new(expected.profile());
    state.rollback(snapshot).expect("matching snapshot profile");

    assert_eq!(state, expected);
}

#[test]
fn snapshot_roundtrip_preserves_nested_alignment_delivery_lifecycle() {
    let mut state = CommandState::default();
    let outer = AlignmentIdentity::new(1);
    let inner = AlignmentIdentity::new(2);

    state.begin_alignment(outer);
    state
        .begin_alignment_cell(outer, templates())
        .expect("outer cell begins");
    state
        .suspend_alignment(outer)
        .expect("outer delivery suspends");
    state.begin_alignment(inner);
    state
        .begin_alignment_cell(inner, templates())
        .expect("inner cell begins");
    let expected = state.clone();
    let snapshot = state.snapshot();

    state
        .finish_alignment(inner)
        .expect("inner delivery can mutate after capture");
    state.rollback(snapshot).expect("matching snapshot profile");

    assert_eq!(state, expected);
    assert_eq!(
        state.publish_summary(),
        Err(CommandSummaryError::AlignmentTemplateActive),
        "an active inner template must keep the nested lifecycle nonquiescent"
    );
}

#[test]
fn quiescent_summary_roundtrip_is_exact_and_deterministic() {
    let expected = populated_quiescent_state();
    let summary = expected
        .publish_summary()
        .expect("the complete quiescent state must be publishable");
    let summary_clone = summary.clone();
    let original_hash = semantic_hash(&summary);

    let mut restored = CommandState::new(expected.profile());
    restored
        .restore_summary(summary)
        .expect("matching summary profile");
    let republished = restored
        .publish_summary()
        .expect("a restored summary must remain quiescent");

    assert_eq!(restored, expected);
    assert_eq!(republished, summary_clone);
    assert_eq!(semantic_hash(&republished), original_hash);
}

#[test]
fn force_eof_is_owned_by_snapshot_identity_and_rollback() {
    let mut state = populated_quiescent_state();
    let false_snapshot = state.snapshot();
    let false_hash = semantic_hash(&false_snapshot);
    state.input.force_eof = true;
    let true_snapshot = state.snapshot();

    assert_ne!(semantic_hash(&true_snapshot), false_hash);
    state.input.force_eof = false;
    state
        .rollback(true_snapshot)
        .expect("non-default input state rolls back");
    assert!(state.input.force_eof);
}

fn assert_rejected(mutate: impl FnOnce(&mut CommandState), expected: CommandSummaryError) {
    let mut state = populated_quiescent_state();
    mutate(&mut state);
    assert_eq!(state.publish_summary(), Err(expected));
}

#[test]
fn summary_rejects_each_scanner_episode() {
    let mut state = populated_quiescent_state();
    let cases = [
        (
            ScannerStatus::Skipping(SkippingContext {
                condition: ConditionId(1),
                warning: ScannerWarning(1),
                skip_line: 0,
                conditional: crate::conditionals::ConditionalKind::IfTrue,
            }),
            CommandSummaryError::ConditionalSkip,
        ),
        (
            ScannerStatus::Matching(MatchingContext {
                macro_name: tex_state::interner::Symbol::testing_new(1),
                builder: ArgumentBuilderId(2),
                warning: ScannerWarning(1),
            }),
            CommandSummaryError::MacroMatch,
        ),
        (
            ScannerStatus::Defining(DefinitionContext {
                target: Some(tex_state::interner::Symbol::testing_new(1)),
                builder: TokenBuilderId(2),
                warning: ScannerWarning(1),
            }),
            CommandSummaryError::DefinitionScan,
        ),
        (
            ScannerStatus::Aligning(AlignmentScanContext {
                alignment: AlignmentId(1),
                builder: TokenBuilderId(2),
                owner: None,
                warning: ScannerWarning(1),
            }),
            CommandSummaryError::AlignmentScan,
        ),
        (
            ScannerStatus::Absorbing(AbsorbingContext {
                owner: Some(tex_state::interner::Symbol::testing_new(1)),
                builder: TokenBuilderId(2),
                warning: ScannerWarning(1),
            }),
            CommandSummaryError::AbsorbingScan,
        ),
    ];
    for (status, expected) in cases {
        let actual = state.with_scanner_status(status, |state| state.publish_summary());
        assert_eq!(actual, Err(expected));
    }
}

#[test]
fn summary_rejects_expansion_alignment_and_live_transients() {
    assert_rejected(
        |state| state.transient.active_expansion_depth = 1,
        CommandSummaryError::ExpansionActive,
    );
    assert_rejected(
        |state| {
            state.alignment.active_cell = Some(ActiveCellDelivery {
                alignment: AlignmentIdentity::new(1),
                templates: templates(),
                u_template_installed: false,
                u_level: None,
                v_level: None,
                delimiter: None,
                omit: false,
                omit_previous_align_state: None,
            });
        },
        CommandSummaryError::AlignmentTemplateActive,
    );
    assert_rejected(
        |state| {
            state.alignment.suspended.push(SuspendedAlignment {
                alignment: AlignmentIdentity::new(1),
                active_cell: None,
            });
        },
        CommandSummaryError::SuspendedAlignment,
    );
    assert_rejected(
        |state| {
            state.transient.builders.push(LiveTokenBuilder {
                identity: 1,
                tokens: tex_state::token::RootedTracedTokenBuffer::default(),
            });
        },
        CommandSummaryError::LiveTokenBuilder,
    );
    assert_rejected(
        |state| state.transient.rollback_roots.push(1),
        CommandSummaryError::LiveRollbackRoot,
    );
}

#[test]
fn snapshot_and_summary_are_owned_static_values() {
    fn assert_owned<T: Clone + Eq + Hash + Send + Sync + 'static>() {}

    assert_owned::<super::CommandStateSnapshot>();
    assert_owned::<CommandSummary>();
}

#[test]
fn snapshot_and_summary_reject_profile_mismatch_without_mutation() {
    let foreign = populated_quiescent_state();
    let snapshot = foreign.snapshot();
    let summary = foreign
        .publish_summary()
        .expect("foreign state is quiescent");
    let mut state = CommandState::new(CommandProfile::TEX82);
    let expected = state.clone();

    let snapshot_error = state
        .rollback(snapshot)
        .expect_err("snapshot from another profile must be rejected");
    assert_eq!(snapshot_error.boundary(), CommandProfileBoundary::Snapshot);
    assert_eq!(state, expected);

    let summary_error = state
        .restore_summary(summary)
        .expect_err("summary from another profile must be rejected");
    assert_eq!(summary_error.boundary(), CommandProfileBoundary::Summary);
    assert_eq!(state, expected);
}

#[test]
fn retained_source_backing_and_partial_line_cursors_restore_without_host_access() {
    let profile = CommandProfile::unicode_extended(crate::CommandDialect::Tex82);
    let mut state = CommandState::new(profile);
    let source = state
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::World,
            Vec::from("é \r\nnext".as_bytes()),
        ))
        .expect("valid immutable Unicode backing");
    state
        .open_registered_source(source)
        .expect("registered source opens");
    let physical = state
        .load_next_source_line(13)
        .expect("first physical line");
    assert_eq!(physical.terminator(), crate::LineTerminator::CrLf);
    let first = state.next_source_character().expect("first scalar");
    assert_eq!(first.code(), crate::CharacterCode::from('é'));
    assert_eq!((first.range().start(), first.range().end()), (0, 2));

    let snapshot = state.snapshot();
    let summary = state
        .publish_summary()
        .expect("a partial source line has no active command episode");
    assert!(
        state
            .next_source_character()
            .expect("endline")
            .is_synthetic()
    );
    state.finish_source_line();
    state
        .load_next_source_line(-1)
        .expect("second physical line");

    state
        .rollback(snapshot)
        .expect("retained backing snapshot restores");
    let restored_endline = state.next_source_character().expect("restored endline");
    assert!(restored_endline.is_synthetic());
    assert_eq!(
        (
            restored_endline.range().start(),
            restored_endline.range().end()
        ),
        (2, 2)
    );

    let mut from_summary = CommandState::new(profile);
    from_summary
        .restore_summary(summary)
        .expect("retained backing summary restores");
    assert!(
        from_summary
            .next_source_character()
            .expect("summary-restored endline")
            .is_synthetic()
    );
}

#[test]
fn format_and_checkpoint_profile_components_reject_mismatch() {
    let profiles = [
        CommandProfile::TEX82,
        CommandProfile::ETEX26,
        CommandProfile::PDFTEX14029,
        CommandProfile::unicode_extended(crate::CommandDialect::Tex82),
        CommandProfile::unicode_extended(crate::CommandDialect::Etex26),
        CommandProfile::unicode_extended(crate::CommandDialect::Pdftex14029),
    ];

    for profile in profiles {
        let state = CommandState::new(profile);
        let matching = profile.fingerprint();
        assert_eq!(state.validate_format_profile(matching), Ok(()));
        assert_eq!(state.validate_checkpoint_profile(matching), Ok(()));
        assert_eq!(
            state.format_profile_fingerprint(),
            CommandProfileFingerprint::from_u64(matching.get())
        );
        assert_eq!(state.checkpoint_profile_fingerprint(), matching);

        for foreign in profiles
            .into_iter()
            .filter(|foreign| *foreign != profile)
            .map(CommandProfile::fingerprint)
        {
            let format_error = state
                .validate_format_profile(foreign)
                .expect_err("foreign format profile must be rejected");
            assert_eq!(format_error.boundary(), CommandProfileBoundary::Format);
            assert_eq!(format_error.expected(), matching);
            assert_eq!(format_error.found(), foreign);

            let checkpoint_error = state
                .validate_checkpoint_profile(foreign)
                .expect_err("foreign checkpoint profile must be rejected");
            assert_eq!(
                checkpoint_error.boundary(),
                CommandProfileBoundary::Checkpoint
            );
            assert_eq!(checkpoint_error.expected(), matching);
            assert_eq!(checkpoint_error.found(), foreign);
        }
    }
}

#[test]
fn nested_input_rollback_retains_the_resulting_typed_condition_stack() {
    let mut state = CommandState::default();
    state.conditions.push(ConditionalKind::IfCase, 419);
    let snapshot = state.snapshot();
    state.conditions.push(ConditionalKind::If, 442);

    state
        .rollback_nested_input_preserving_conditions(snapshot)
        .expect("nested input rollback preserves profile");

    assert_eq!(state.conditions.frames.len(), 2);
    assert_eq!(state.conditions.frames[0].kind, ConditionalKind::IfCase);
    assert_eq!(state.conditions.frames[0].source_line, 419);
    assert_eq!(state.conditions.frames[1].kind, ConditionalKind::If);
    assert_eq!(state.conditions.frames[1].source_line, 442);
}
