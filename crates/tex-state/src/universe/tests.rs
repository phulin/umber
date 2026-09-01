use super::{UniverseError, with_universe};
use crate::env::AssignmentScope;
use crate::env::banks::IntParam;
use crate::hyphenation::{ExceptionSpec, PatternSpec};
use crate::interner::InternerBudget;
use crate::meaning::{Meaning, MeaningFlags, MeaningWord, ResolvedMeaning};
use crate::node::{BoxLr, BoxNode, BoxNodeFields, Node, Sign};
use crate::node_arena::NodeArenaError;
use crate::token::{Token, TokenWord};
use crate::{GroupKind, ParagraphShapeLine, PenaltyArrayKind, StateError};
use std::path::PathBuf;
use tex_arith::{GlueSetRatio, Scaled};
use tex_fonts::{FontMetrics, LoadedFont};

fn budget() -> InternerBudget {
    InternerBudget::new(32, 32, 1024).expect("budget")
}

fn test_font(name: &str) -> LoadedFont {
    LoadedFont::new(
        name,
        PathBuf::from(format!("/fonts/{name}.tfm")),
        tex_fonts::font_content_hash(name.as_bytes()),
        0x1234_5678,
        Scaled::from_raw(10 * Scaled::UNITY),
        Scaled::from_raw(10 * Scaled::UNITY),
        vec![Scaled::from_raw(0); 7],
        FontMetrics::default(),
    )
}

#[test]
fn command_episode_admits_session_and_generation_once() {
    with_universe(budget(), |universe| {
        let symbol = universe.intern("alpha").expect("intern");
        universe
            .assign_meaning(
                symbol,
                MeaningWord::from_static(Meaning::Relax),
                AssignmentScope::Global,
            )
            .expect("assign");

        let context = universe.command_context().expect("admit episode");
        assert_eq!(context.resolve_symbol(symbol), Ok("alpha"));
        assert_eq!(
            context.meaning(symbol.symbol()),
            ResolvedMeaning::Static(Meaning::Relax)
        );
    })
    .expect("universe allocation");
}

#[test]
fn runtime_checkpoint_fork_moves_the_checkpoint_bank_without_new_payload_owners() {
    with_universe(budget(), |universe| {
        let symbol = universe.intern("shared").expect("intern symbol");
        let words = [TokenWord::pack(Token::frozen_relax())];
        let definition = universe
            .allocate_definition(&[], &words)
            .expect("definition");
        let tokens = universe.allocate_token_list(&words).expect("token list");
        universe
            .assign_meaning(
                symbol,
                MeaningWord::macro_definition(MeaningFlags::from_bits(0), definition),
                AssignmentScope::Global,
            )
            .expect("assign macro");
        universe
            .assign_token_register(0, Some(tokens.clone()), AssignmentScope::Global)
            .expect("assign token register");
        let checkpoint = universe.runtime_checkpoint().expect("checkpoint");
        let definition_owners = definition.semantic_owner_count();
        let token_owners = tokens.semantic_owner_count();

        let mut fork = universe
            .fork_runtime_checkpoint(&checkpoint)
            .expect("checkpoint fork");
        assert_eq!(definition.semantic_owner_count(), definition_owners);
        assert_eq!(tokens.semantic_owner_count(), token_owners);
        universe.reject_checkpoint_candidate(&mut fork);

        assert_eq!(definition.semantic_owner_count(), definition_owners);
        assert_eq!(tokens.semantic_owner_count(), token_owners);
    })
    .expect("universe allocation");
}

#[test]
fn runtime_identity_demand_publishes_every_authoritative_owner_root() {
    with_universe(budget(), |universe| {
        let ordinary = universe.runtime_checkpoint().expect("ordinary checkpoint");
        assert_eq!(
            ordinary.reachable_state_identity_roots(),
            crate::RuntimeCheckpointIdentityRoots::default()
        );

        universe.enable_reachable_state_identity();
        let demanded = universe
            .runtime_checkpoint_with_page_roots_and_identity(false, true)
            .expect("identity-demanded checkpoint");
        let roots = demanded.reachable_state_identity_roots();
        assert!(roots.pdf().is_some(), "PDF publishes a maintained root");
        assert!(roots.page().is_some(), "page publishes a maintained root");
        assert!(roots.world().is_some());
        assert!(roots.hyphenation().is_some());
        assert!(roots.dependency().is_some());
        assert!(roots.source().is_some());
        assert!(roots.font().is_some());
        assert!(roots.core().is_some());
    })
    .expect("universe allocation");
}

#[test]
fn late_runtime_identity_selection_fails_closed_for_missed_core_mutation() {
    with_universe(budget(), |universe| {
        universe
            .assign_count(17, 41, AssignmentScope::Global)
            .expect("pre-demand mutation");
        universe.enable_reachable_state_identity();
        let roots = universe
            .runtime_checkpoint_with_page_roots_and_identity(false, true)
            .expect("identity-demanded checkpoint")
            .reachable_state_identity_roots();
        assert_eq!(roots.core(), None, "missed owner history stays unavailable");
        assert!(roots.page().is_some());
        assert!(roots.world().is_some());
    })
    .expect("universe allocation");
}

#[test]
fn maintained_runtime_roots_change_at_each_owner_mutation_barrier() {
    with_universe(budget(), |universe| {
        universe.enable_reachable_state_identity();
        let baseline = universe
            .runtime_checkpoint_with_page_roots_and_identity(false, true)
            .expect("baseline checkpoint")
            .reachable_state_identity_roots();

        universe
            .assign_count(9, 73, AssignmentScope::Global)
            .expect("core mutation");
        let after_core = universe
            .runtime_checkpoint_with_page_roots_and_identity(false, true)
            .expect("core checkpoint")
            .reachable_state_identity_roots();
        assert_ne!(after_core.core(), baseline.core());

        universe
            .world_mut()
            .write_text(crate::PrintSink::Terminal, "identity");
        let after_world = universe
            .runtime_checkpoint_with_page_roots_and_identity(false, true)
            .expect("World checkpoint")
            .reachable_state_identity_roots();
        assert_ne!(after_world.world(), baseline.world());

        universe
            .command_context()
            .expect("command admission")
            .add_hyphenation_exception_for_language(
                3,
                ExceptionSpec {
                    word: "semantic".to_owned(),
                    positions: vec![3],
                },
            );
        let after_hyphenation = universe
            .runtime_checkpoint_with_page_roots_and_identity(false, true)
            .expect("hyphenation checkpoint")
            .reachable_state_identity_roots();
        assert_ne!(after_hyphenation.hyphenation(), baseline.hyphenation());

        let token = universe
            .begin_dependency_region()
            .expect("dependency region");
        universe
            .abandon_dependency_region(token)
            .expect("dependency region closes");
        let after_dependency = universe
            .runtime_checkpoint_with_page_roots_and_identity(false, true)
            .expect("dependency checkpoint")
            .reachable_state_identity_roots();
        assert_ne!(after_dependency.dependency(), baseline.dependency());

        universe
            .command_context()
            .expect("command admission")
            .register_source(
                crate::input::SourceId::new(91),
                crate::source_map::SourceDescriptor::named_generated(
                    "semantic.tex",
                    std::sync::Arc::from(&b"semantic source"[..]),
                ),
            )
            .expect("source registration");
        let after_source = universe
            .runtime_checkpoint_with_page_roots_and_identity(false, true)
            .expect("source checkpoint")
            .reachable_state_identity_roots();
        assert_ne!(after_source.source(), baseline.source());

        universe
            .command_context()
            .expect("command admission")
            .intern_font(test_font("identityfont"));
        let after_font = universe
            .runtime_checkpoint_with_page_roots_and_identity(false, true)
            .expect("font checkpoint")
            .reachable_state_identity_roots();
        assert_ne!(after_font.font(), baseline.font());
    })
    .expect("universe allocation");
}

#[test]
fn equivalent_macro_definitions_keep_the_same_core_identity() {
    with_universe(budget(), |universe| {
        assert!(universe.enable_reachable_state_identity());
        let symbol = universe.intern("stablemacro").expect("macro symbol");
        let body = [TokenWord::pack(Token::frozen_relax())];
        let first = universe
            .allocate_definition(&[], &body)
            .expect("first definition");
        universe
            .assign_meaning(
                symbol,
                MeaningWord::macro_definition(MeaningFlags::EMPTY, first),
                AssignmentScope::Global,
            )
            .expect("first meaning");
        let first_root = universe
            .runtime_checkpoint_with_page_roots_and_identity(false, true)
            .expect("first checkpoint")
            .reachable_state_identity_roots()
            .core();

        let _unrelated = universe
            .allocate_definition(
                &[],
                &[TokenWord::pack(Token::Char {
                    ch: 'x',
                    cat: crate::token::Catcode::Letter,
                })],
            )
            .expect("unrelated definition");
        let equivalent = universe
            .allocate_definition(&[], &body)
            .expect("equivalent definition");
        universe
            .assign_meaning(
                symbol,
                MeaningWord::macro_definition(MeaningFlags::EMPTY, equivalent),
                AssignmentScope::Global,
            )
            .expect("equivalent meaning");
        let equivalent_root = universe
            .runtime_checkpoint_with_page_roots_and_identity(false, true)
            .expect("equivalent checkpoint")
            .reachable_state_identity_roots()
            .core();

        assert_eq!(first_root, equivalent_root);
    })
    .expect("universe allocation");
}

#[test]
fn checkpoint_definition_row_reuse_restores_the_matching_content_identity() {
    with_universe(budget(), |universe| {
        assert!(universe.enable_reachable_state_identity());
        let symbol = universe.intern("rowreusemacro").expect("macro symbol");
        let baseline = universe
            .allocate_definition(&[], &[TokenWord::pack(Token::frozen_relax())])
            .expect("baseline definition");
        universe
            .assign_meaning(
                symbol,
                MeaningWord::macro_definition(MeaningFlags::EMPTY, baseline),
                AssignmentScope::Global,
            )
            .expect("baseline meaning");
        let checkpoint = universe.runtime_checkpoint().expect("baseline checkpoint");

        let accepted = universe
            .allocate_definition(
                &[],
                &[TokenWord::pack(Token::Char {
                    ch: 'a',
                    cat: crate::token::Catcode::Letter,
                })],
            )
            .expect("accepted definition");
        universe
            .assign_meaning(
                symbol,
                MeaningWord::macro_definition(MeaningFlags::EMPTY, accepted),
                AssignmentScope::Global,
            )
            .expect("accepted meaning");
        let accepted_root = universe
            .core
            .as_ref()
            .expect("accepted core")
            .reachable_state_identity_root()
            .expect("accepted identity");

        let mut candidate = universe
            .fork_runtime_checkpoint(&checkpoint)
            .expect("candidate from baseline");
        let replacement = candidate
            .allocate_definition(
                &[],
                &[TokenWord::pack(Token::Char {
                    ch: 'b',
                    cat: crate::token::Catcode::Letter,
                })],
            )
            .expect("candidate definition");
        assert_eq!(replacement, accepted, "candidate reuses the detached row");
        candidate
            .core
            .as_mut()
            .expect("candidate core")
            .admit_mut()
            .expect("candidate admission")
            .assign_meaning(
                symbol.symbol(),
                MeaningWord::macro_definition(MeaningFlags::EMPTY, replacement),
                AssignmentScope::Global,
            )
            .expect("candidate meaning");
        let candidate_root = candidate
            .core
            .as_ref()
            .expect("candidate core")
            .reachable_state_identity_root()
            .expect("candidate identity");
        assert_ne!(candidate_root, accepted_root);

        universe.reject_checkpoint_candidate(&mut candidate);
        let restored_root = universe
            .core
            .as_ref()
            .expect("restored core")
            .reachable_state_identity_root()
            .expect("restored identity");
        assert_eq!(restored_root, accepted_root);
    })
    .expect("universe allocation");
}

#[test]
fn rejected_checkpoint_loan_invalidates_candidate_coordinates_before_retry() {
    with_universe(budget(), |universe| {
        for index in 0..16 {
            universe.register_primitive_meaning(&format!("primitive{index}"), Meaning::Relax);
            universe
                .assign_count(index, i32::from(index), AssignmentScope::Global)
                .expect("baseline dense row");
            universe
                .begin_group(GroupKind::Simple, u32::from(index))
                .expect("baseline save segment");
        }
        let checkpoint = universe.runtime_checkpoint().expect("early checkpoint");
        for index in 0..16 {
            universe
                .assign_count(index, -i32::from(index), AssignmentScope::Local)
                .expect("accepted suffix");
            let _ = universe.publish_page_nodes(&[Node::Penalty(-i32::from(index))]);
        }

        let mut candidate = universe
            .fork_runtime_checkpoint(&checkpoint)
            .expect("loan exact checkpoint bank");
        let first_node = candidate.publish_page_nodes(&[Node::Penalty(91)]);
        let first_glue = candidate
            .allocate_glue(crate::glue::GlueSpec::ZERO)
            .expect("candidate glue");
        let first_provenance = candidate
            .allocate_provenance(crate::provenance::OriginRecord::UnknownBootstrap)
            .expect("candidate provenance");
        candidate
            .assign_count(50_000, 91, AssignmentScope::Global)
            .expect("candidate dense suffix");
        universe.reject_checkpoint_candidate(&mut candidate);

        let mut retry = universe
            .fork_runtime_checkpoint(&checkpoint)
            .expect("reloan returned checkpoint bank");
        let retry_node = retry.publish_page_nodes(&[Node::Penalty(91)]);
        assert_ne!(
            retry_node, first_node,
            "released chunks advance their generation before retry"
        );
        assert!(retry.page_node_list(first_node).is_err());
        assert_eq!(
            retry
                .page_node_list(retry_node)
                .expect("retry coordinate is live")
                .owned_node(0),
            Some(&Node::Penalty(91))
        );
        assert_eq!(
            retry
                .allocate_glue(crate::glue::GlueSpec::ZERO)
                .expect("retry glue"),
            first_glue
        );
        assert_eq!(
            retry
                .allocate_provenance(crate::provenance::OriginRecord::UnknownBootstrap)
                .expect("retry provenance"),
            first_provenance
        );
        assert_eq!(retry.primitive_registry_len(), 16);
        assert_eq!(retry.count(50_000).expect("restored dense sentinel"), 0);
        universe.reject_checkpoint_candidate(&mut retry);
    })
    .expect("universe allocation");
}

#[test]
fn runtime_checkpoint_fork_resets_newer_retained_page_bound() {
    with_universe(budget(), |universe| {
        let checkpoint = universe.runtime_checkpoint().expect("empty checkpoint");
        let _ = universe.publish_page_nodes(&[Node::Penalty(7)]);
        universe
            .runtime_checkpoint_with_page_roots(true)
            .expect("newer retained page bound");

        let mut fork = universe
            .fork_runtime_checkpoint(&checkpoint)
            .expect("older checkpoint fork");
        fork.runtime_checkpoint()
            .expect("forked retained bound addresses its truncated page arena");
        universe.reject_checkpoint_candidate(&mut fork);
    })
    .expect("universe allocation");
}

#[test]
fn runtime_checkpoint_hyphenation_restore_and_fork_isolate_mutable_state() {
    with_universe(budget(), |universe| {
        {
            let mut context = universe.command_context().expect("hyphenation context");
            context
                .add_hyphenation_pattern_for_language(
                    7,
                    PatternSpec {
                        letters: "hyphen".chars().collect(),
                        values: vec![0, 2, 0, 3, 0, 0, 0],
                    },
                )
                .expect("pattern fits");
            context.add_hyphenation_exception_for_language(
                7,
                ExceptionSpec {
                    word: "baseline".to_owned(),
                    positions: vec![4],
                },
            );
            context.save_hyphenation_codes(7, [('A', 'a')]);
            context.close_hyphenation_patterns();
        }
        let checkpoint = universe.runtime_checkpoint().expect("hyphen checkpoint");

        {
            let mut context = universe.command_context().expect("speculative context");
            context.add_hyphenation_exception_for_language(
                7,
                ExceptionSpec {
                    word: "hyphen".to_owned(),
                    positions: vec![4],
                },
            );
            context.save_hyphenation_codes(7, [('A', 'z')]);
        }
        universe
            .restore_runtime_checkpoint_with_roots(&checkpoint, || {})
            .expect("restore hyphen checkpoint");
        {
            let context = universe.command_context().expect("restored context");
            assert_eq!(
                context.hyphen_positions_for_language(7, "hyphen", 0, 0),
                vec![3],
                "the initialized trie survives restore"
            );
            assert_eq!(
                context.hyphen_positions_for_language(7, "baseline", 0, 0),
                vec![4],
                "the checkpointed exception survives restore"
            );
            assert_eq!(context.saved_hyphenation_code(7, 'A'), Some(Some('a')));
        }

        let mut fork = universe
            .fork_runtime_checkpoint(&checkpoint)
            .expect("fork hyphen checkpoint");
        {
            fork.hyphenation.add_exception_for_language(
                7,
                ExceptionSpec {
                    word: "hyphen".to_owned(),
                    positions: vec![2],
                },
            );
            fork.hyphenation.save_hyphen_codes(7, [('A', 'f')]);
            assert_eq!(
                fork.hyphenation
                    .hyphen_positions_for_language(7, "hyphen", 0, 0),
                vec![2]
            );
        }
        universe.reject_checkpoint_candidate(&mut fork);
        assert_eq!(
            universe
                .hyphenation
                .hyphen_positions_for_language(7, "hyphen", 0, 0),
            vec![3],
            "rejected fork exceptions do not mutate accepted state"
        );
        assert_eq!(
            universe.hyphenation.saved_hyphen_code(7, 'A'),
            Some(Some('a'))
        );
    })
    .expect("universe allocation");
}

#[test]
fn primitive_installation_observes_only_canonical_multiletter_lookups() {
    with_universe(budget(), |universe| {
        universe.register_primitive_meaning("frozenonly", Meaning::Relax);
        universe.install_primitive_meaning("x", Meaning::Relax);
        assert_eq!(
            universe
                .command_context()
                .expect("usage after excluded names")
                .detach_engine_usage_statistics()
                .control_sequences,
            0,
            "frozen registry rows and single-character primitives use fixed slots"
        );

        universe.install_primitive_meaning("visible", Meaning::Relax);
        universe.install_primitive_meaning("visible", Meaning::Relax);
        assert_eq!(
            universe
                .command_context()
                .expect("usage after repeated primitive installation")
                .detach_engine_usage_statistics()
                .control_sequences,
            1,
            "§265 creation increments once and reuse preserves the ledger"
        );
    })
    .expect("universe allocation");
}

#[test]
fn primitive_handle_is_direct_immutable_and_registry_scoped() {
    with_universe(budget(), |universe| {
        universe.install_primitive_meaning("visible", Meaning::Relax);
        let symbol = universe.intern("visible").expect("primitive symbol");
        let handle = universe
            .primitive_handle("visible")
            .expect("static primitive handle");
        assert_eq!(
            universe.resolve_primitive_handle(handle),
            Some(Meaning::Relax)
        );
        assert_eq!(
            universe
                .command_context()
                .expect("direct command-context lookup")
                .resolve_primitive_handle(handle),
            Some(Meaning::Relax),
        );

        universe
            .assign_meaning(
                symbol,
                MeaningWord::from_static(Meaning::Undefined),
                AssignmentScope::Global,
            )
            .expect("redefine visible control sequence");
        assert_eq!(
            universe
                .command_context()
                .expect("meaning lookup")
                .meaning(symbol.symbol()),
            ResolvedMeaning::Static(Meaning::Undefined),
            "the packed primitive handle must not cache the mutable eqtb cell"
        );
        assert_eq!(
            universe.resolve_primitive_handle(handle),
            Some(Meaning::Relax),
            "the immutable original primitive remains available"
        );

        universe.register_primitive_meaning("later", Meaning::Undefined);
        assert_eq!(
            universe.resolve_primitive_handle(handle),
            None,
            "extending a registry invalidates handles issued before completion"
        );
        assert_eq!(
            universe
                .command_context()
                .expect("stale command-context lookup")
                .resolve_primitive_handle(handle),
            None,
            "command admission enforces the same completed-registry extent",
        );
        let rebound = universe
            .primitive_handle("visible")
            .expect("rebound complete-registry handle");
        assert_eq!(
            universe.resolve_primitive_handle(rebound),
            Some(Meaning::Relax)
        );
    })
    .expect("universe allocation");
}

#[test]
fn csname_relaxes_previously_interned_undefined_control_sequence_only() {
    with_universe(budget(), |universe| {
        let undefined = universe.intern("latent").expect("intern undefined symbol");
        let defined = universe.intern("defined").expect("intern defined symbol");
        universe
            .assign_meaning(
                defined,
                MeaningWord::from_static(Meaning::IntParam(IntParam::MAG.raw())),
                AssignmentScope::Global,
            )
            .expect("assign defined meaning");

        let mut context = universe.command_context().expect("admit episode");
        context.begin_group(GroupKind::Simple, 1).expect("group");

        assert_eq!(
            context.intern_relaxed_control_sequence("latent"),
            undefined.symbol()
        );
        assert_eq!(
            context.meaning(undefined.symbol()),
            ResolvedMeaning::Static(Meaning::Relax)
        );
        assert_eq!(
            context.intern_relaxed_control_sequence("defined"),
            defined.symbol()
        );
        assert_eq!(
            context.meaning(defined.symbol()),
            ResolvedMeaning::Static(Meaning::IntParam(IntParam::MAG.raw()))
        );

        context
            .end_group(GroupKind::Simple)
            .expect("restore local implicit relaxation");
        assert_eq!(
            context.meaning(undefined.symbol()),
            ResolvedMeaning::Static(Meaning::Undefined)
        );
    })
    .expect("universe allocation");
}

#[test]
fn csname_creation_observes_hash_occupancy_once_across_group_restore() {
    with_universe(budget(), |universe| {
        let mut context = universe.command_context().expect("admit episode");
        let constructed = context.intern_control_sequence("one \\csname");
        assert_eq!(
            context.detach_engine_usage_statistics().control_sequences,
            0
        );
        context.begin_group(GroupKind::Simple, 1).expect("group");
        assert_eq!(
            context.intern_relaxed_control_sequence("one \\csname"),
            constructed
        );
        assert_eq!(
            context.meaning(constructed),
            ResolvedMeaning::Static(Meaning::Relax)
        );
        assert_eq!(
            context.detach_engine_usage_statistics().control_sequences,
            1
        );
        assert_eq!(
            context.intern_relaxed_control_sequence("one \\csname"),
            constructed
        );
        context
            .end_group(GroupKind::Simple)
            .expect("restore implicit relaxation");
        assert_eq!(
            context.meaning(constructed),
            ResolvedMeaning::Static(Meaning::Undefined)
        );
        assert_eq!(
            context.detach_engine_usage_statistics().control_sequences,
            1,
            "§§256/372 retain the created name after meaning rollback"
        );
    })
    .expect("universe allocation");
}

#[cfg(not(feature = "profiling"))]
#[test]
fn warmed_control_sequence_interning_allocates_nothing() {
    with_universe(budget(), |universe| {
        let mut context = universe.command_context().expect("admit episode");
        let expected = context.intern_hash_control_sequence("warmed-control-sequence");
        const OWNER: usize = 14;
        let before = umber_hot_core_allocator::thread_measurement(OWNER);
        let stable =
            {
                let _scope = umber_hot_core_allocator::scope(OWNER);
                (0..4_096).all(|_| {
                    std::hint::black_box(context.intern_hash_control_sequence(
                        std::hint::black_box("warmed-control-sequence"),
                    )) == expected
                })
            };
        let after = umber_hot_core_allocator::thread_measurement(OWNER);

        assert!(stable);
        assert_eq!(after.calls - before.calls, 0);
        assert_eq!(after.requested_bytes - before.requested_bytes, 0);
    })
    .expect("universe allocation");
}

/// TeX82 §288: `mag_set` freezes the first prepared magnification, corrects
/// an incompatible later assignment globally, and belongs to the checkpointed
/// job session rather than to a reusable format image.
#[test]
fn prepared_magnification_is_job_scoped_and_checkpointed() {
    with_universe(budget(), |universe| {
        {
            let mut context = universe.command_context().expect("context");
            context
                .assign_int_param(IntParam::MAG, 1_200, AssignmentScope::Global)
                .expect("mag assignment");
        }
        let checkpoint = universe.runtime_checkpoint().expect("checkpoint");
        assert_eq!(
            universe.command_context().expect("context").prepare_mag(),
            (1_200, None)
        );
        {
            let mut context = universe.command_context().expect("context");
            context
                .assign_int_param(IntParam::MAG, 2_000, AssignmentScope::Global)
                .expect("mag assignment");
            assert_eq!(
                context.prepare_mag(),
                (
                    1_200,
                    Some(crate::PrepareMagDiagnostic::IncompatibleMagnification {
                        attempted: 2_000,
                        retained: 1_200,
                    })
                )
            );
            assert_eq!(context.int_param(IntParam::MAG), 1_200);
        }

        universe
            .restore_runtime_checkpoint_with_roots(&checkpoint, || {})
            .expect("restore checkpoint");
        {
            let mut transaction = universe.begin_shipout();
            assert_eq!(
                transaction
                    .command_context()
                    .expect("context")
                    .prepare_mag(),
                (1_200, None)
            );
        }
        {
            let mut context = universe.command_context().expect("context");
            context
                .assign_int_param(IntParam::MAG, 2_000, AssignmentScope::Global)
                .expect("mag assignment");
            assert_eq!(context.prepare_mag(), (2_000, None));
        }
    })
    .expect("universe allocation");
}

#[test]
fn runtime_checkpoint_restores_string_pool_accounting() {
    with_universe(budget(), |universe| {
        let initial = universe
            .command_context()
            .expect("initial context")
            .detach_engine_usage_statistics();
        let checkpoint = universe.runtime_checkpoint().expect("checkpoint");
        universe.set_engine_capacity_profile(crate::EngineCapacityProfile::Texlive2026);
        let expanded = universe
            .command_context()
            .expect("expanded context")
            .detach_engine_usage_statistics();
        assert_eq!(
            (expanded.capacity_profile, expanded.memory_word_capacity),
            (crate::EngineCapacityProfile::Texlive2026, 5_000_000)
        );
        universe
            .command_context()
            .expect("context")
            .make_string_pool_string("speculative");
        assert_eq!(
            universe
                .command_context()
                .expect("context")
                .detach_engine_usage_statistics()
                .strings,
            1
        );
        universe
            .restore_runtime_checkpoint_with_roots(&checkpoint, || {})
            .expect("restore checkpoint");
        let mut context = universe.command_context().expect("context");
        assert_eq!(context.detach_engine_usage_statistics(), initial);
        context.slow_make_string_pool_string("speculative");
        assert_eq!(
            context.detach_engine_usage_statistics().strings,
            1,
            "rollback removes speculative membership as well as coordinates"
        );
    })
    .expect("universe allocation");
}

#[test]
fn runtime_checkpoint_restores_main_memory_extents() {
    with_universe(budget(), |universe| {
        let checkpoint = universe.runtime_checkpoint().expect("checkpoint");
        universe
            .command_context()
            .expect("context")
            .observe_transient_token_words(600);
        assert_eq!(
            universe
                .command_context()
                .expect("context")
                .detach_engine_usage_statistics()
                .memory_words,
            1_635
        );
        universe
            .restore_runtime_checkpoint_with_roots(&checkpoint, || {})
            .expect("restore checkpoint");
        assert_eq!(
            universe
                .command_context()
                .expect("context")
                .detach_engine_usage_statistics()
                .memory_words,
            1_035
        );
    })
    .expect("universe allocation");
}

#[test]
fn font_info_usage_joins_immutable_metrics_and_mutable_parameter_growth() {
    // TeX82 §§549/552/565 advance one shared `fmem_ptr` for the null font,
    // each TFM's complete table, and §580 parameter growth. Umber keeps the
    // immutable metrics and mutable parameters in distinct owners.
    with_universe(budget(), |universe| {
        let checkpoint = universe
            .runtime_checkpoint()
            .expect("font prefix checkpoint");
        let loaded = test_font("usagefont").with_font_info_words(19_991);
        let duplicate = loaded.clone();
        let font = universe
            .command_context()
            .expect("context")
            .intern_font(loaded);
        assert_eq!(
            universe
                .command_context()
                .expect("context")
                .intern_font(duplicate),
            font,
            "an existing load consumes no second font-info extent"
        );
        {
            let mut context = universe.command_context().expect("context");
            assert_eq!(
                context.detach_engine_usage_statistics().font_info_words,
                19_998
            );
            context
                .set_font_dimen(font, 9, Scaled::from_raw(9))
                .expect("two parameter words fit exactly");
            assert_eq!(
                context.detach_engine_usage_statistics().font_info_words,
                20_000
            );
            assert_eq!(
                context.set_font_dimen(font, 10, Scaled::from_raw(10)),
                Err(20_000)
            );
            assert_eq!(
                context.detach_engine_usage_statistics().font_info_words,
                20_000
            );
        }
        universe
            .restore_runtime_checkpoint_with_roots(&checkpoint, || {})
            .expect("restore font prefix");
        assert_eq!(
            universe
                .command_context()
                .expect("restored context")
                .detach_engine_usage_statistics()
                .font_info_words,
            7,
            "rollback removes the exact immutable and mutable font suffix"
        );
    })
    .expect("universe allocation");
}

#[test]
fn universe_terminal_input_cursor_replays_only_its_caller_world() {
    let position = with_universe(budget(), |universe| {
        universe
            .world_mut()
            .push_memory_terminal_line("replay")
            .expect("memory terminal input");
        let position = universe.capture_terminal_input_position();
        assert_eq!(
            universe.world_mut().read_terminal_line().expect("read"),
            Some("replay".to_owned())
        );
        universe
            .restore_terminal_input_position(position)
            .expect("same World cursor");
        assert_eq!(
            universe.world_mut().read_terminal_line().expect("replay"),
            Some("replay".to_owned())
        );
        position
    })
    .expect("source universe");

    with_universe(budget(), |universe| {
        assert!(universe.restore_terminal_input_position(position).is_err());
    })
    .expect("foreign universe");
}

#[test]
fn font_meaning_retains_the_exact_live_timeline_coordinate() {
    with_universe(budget(), |universe| {
        let symbol = universe.intern("bodyfont").expect("intern font selector");
        let font = universe
            .command_context()
            .expect("context")
            .intern_font(test_font("bodyfont"));
        assert_eq!(
            universe.command_context().expect("context").font_name(font),
            "bodyfont"
        );

        universe
            .assign_meaning(
                symbol,
                MeaningWord::from_static(Meaning::Font(font)),
                AssignmentScope::Global,
            )
            .expect("assign exact font meaning");
        let context = universe.command_context().expect("context");
        assert_eq!(
            context.meaning(symbol.symbol()),
            ResolvedMeaning::Static(Meaning::Font(font))
        );
        assert_eq!(context.font_name(font), "bodyfont");
    })
    .expect("universe allocation");
}

#[test]
fn null_font_and_scalar_meanings_keep_their_existing_round_trips() {
    with_universe(budget(), |universe| {
        let null = universe.intern("null").expect("intern null selector");
        let scalar = universe.intern("scalar").expect("intern scalar selector");
        universe
            .assign_meaning(
                null,
                MeaningWord::from_static(Meaning::Font(crate::font::NULL_FONT)),
                AssignmentScope::Global,
            )
            .expect("assign null font");
        universe
            .assign_meaning(
                scalar,
                MeaningWord::from_static(Meaning::CharGiven('A')),
                AssignmentScope::Global,
            )
            .expect("assign scalar");
        let context = universe.command_context().expect("context");
        assert_eq!(
            context.meaning(null.symbol()),
            ResolvedMeaning::Static(Meaning::Font(crate::font::NULL_FONT))
        );
        assert_eq!(
            context.meaning(scalar.symbol()),
            ResolvedMeaning::Static(Meaning::CharGiven('A'))
        );
    })
    .expect("universe allocation");
}

#[test]
fn rollback_never_recycles_an_interned_symbol() {
    with_universe(budget(), |universe| {
        let first = universe.intern("first").expect("intern first");
        let cursor = universe.begin_state_operation().expect("operation");
        let second = universe.intern("second").expect("intern second");
        assert_eq!(
            universe
                .command_context()
                .expect("usage before rollback")
                .detach_engine_usage_statistics()
                .control_sequences,
            2
        );
        universe.restore_state(cursor).expect("state rollback");

        assert_eq!(universe.resolve_symbol(first), Ok("first"));
        assert_eq!(universe.resolve_symbol(second), Ok("second"));
        assert_eq!(universe.intern("second"), Ok(second));
        assert_eq!(
            universe
                .command_context()
                .expect("usage after rollback")
                .detach_engine_usage_statistics()
                .control_sequences,
            2,
            "§256 occupancy survives state rollback and repeated lookup"
        );
    })
    .expect("universe allocation");
}

#[test]
fn whole_session_retirement_rejects_future_admission() {
    with_universe(budget(), |universe| {
        universe.intern("retained").expect("intern");
        let retired = universe.retire().expect("retire");
        assert_eq!(retired.interner_usage().control_sequence_names(), 1);
        assert!(universe.is_retired());
        assert_eq!(
            universe.command_context().err(),
            Some(UniverseError::Retired)
        );
        assert_eq!(universe.intern("late"), Err(UniverseError::Retired));
    })
    .expect("universe allocation");
}

#[test]
fn foreign_session_symbols_are_rejected_before_dense_access() {
    let mut foreign = None;
    with_universe(budget(), |universe| {
        foreign = Some(universe.intern("foreign").expect("intern"));
    })
    .expect("first universe");

    with_universe(budget(), |universe| {
        let local = universe.intern("local").expect("intern local");
        let context = universe.command_context().expect("context");
        assert_eq!(context.resolve_symbol(local), Ok("local"));
        assert!(
            context
                .resolve_symbol(foreign.expect("foreign id"))
                .is_err()
        );
    })
    .expect("second universe");
}

#[test]
fn retained_state_checkpoint_restores_dense_roots_before_arena_suffixes() {
    with_universe(budget(), |universe| {
        universe
            .assign_count(0, 10, AssignmentScope::Global)
            .expect("baseline count");
        let checkpoint = universe.state_checkpoint().expect("checkpoint");
        let rejected = universe.publish_page_nodes(&[Node::Penalty(99)]);
        universe
            .assign_count(0, 20, AssignmentScope::Global)
            .expect("candidate count");

        universe
            .restore_state_checkpoint(&checkpoint)
            .expect("restore checkpoint");

        assert_eq!(
            universe
                .command_context()
                .expect("test fixture is valid")
                .count(0)
                .expect("test fixture is valid"),
            10
        );
        assert_eq!(
            universe
                .page_node_list(rejected)
                .expect_err("invalid test fixture is rejected"),
            NodeArenaError::InvalidList
        );
        assert_eq!(
            universe.retire(),
            Err(UniverseError::State(crate::StateError::GenerationInUse))
        );
        drop(checkpoint);
        universe.retire().expect("last coarse owner released");
    })
    .expect("universe allocation");
}

#[test]
fn rootless_runtime_checkpoint_releases_only_the_unreachable_suffix() {
    with_universe(budget(), |universe| {
        let retained = universe.publish_page_nodes(&[Node::Penalty(3)]);
        universe
            .command_context()
            .expect("context")
            .append_page_contribution(Node::HList(BoxNode::new(BoxNodeFields {
                width: Scaled::from_raw(0),
                height: Scaled::from_raw(0),
                depth: Scaled::from_raw(0),
                shift: Scaled::from_raw(0),
                box_lr: BoxLr::Normal,
                glue_set: GlueSetRatio::ZERO,
                glue_sign: Sign::Normal,
                glue_order: crate::glue::Order::Normal,
                children: retained,
            })));
        let partial = universe.runtime_checkpoint().expect("partial checkpoint");
        assert_eq!(
            universe.page_node_rows(),
            2,
            "the child and enclosing contribution are distinct canonical payloads"
        );

        let detached = universe
            .command_context()
            .expect("context")
            .pop_page_contribution_front()
            .expect("page contribution");
        universe
            .command_context()
            .expect("context")
            .discard_page_node(detached);
        let discarded_a = universe.publish_page_nodes(&[Node::Penalty(7)]);
        let discarded_b = universe.publish_page_nodes(&[Node::Penalty(9)]);
        assert_eq!(universe.page_node_rows(), 4);
        let rootless = universe.runtime_checkpoint().expect("rootless checkpoint");
        assert_eq!(
            universe.page_node_rows(),
            2,
            "the retained checkpoint prefix survives while incidental suffix rows retire"
        );
        assert!(universe.page_node_list(discarded_a).is_err());
        assert!(universe.page_node_list(discarded_b).is_err());

        for checkpoint in [&partial, &rootless, &partial, &rootless] {
            let mut candidate = universe
                .fork_runtime_checkpoint(checkpoint)
                .expect("either sibling mark can seed a candidate");
            universe.reject_checkpoint_candidate(&mut candidate);
        }
        universe
            .restore_runtime_checkpoint_with_roots(&partial, || {})
            .expect("accepted restore keeps the retained prefix exact");
        assert!(universe.page_node_list(retained).is_ok());
        assert!(universe.page_node_list(discarded_a).is_err());
        assert!(universe.page_node_list(discarded_b).is_err());
        assert_eq!(
            universe
                .command_context()
                .expect("context")
                .page_contributions()
                .len(),
            1,
            "the partial-page checkpoint remains restorable after rootless siblings"
        );
    })
    .expect("universe allocation");
}

#[test]
fn external_mode_roots_block_rootless_page_suffix_release() {
    with_universe(budget(), |universe| {
        let unreachable = universe.publish_page_nodes(&[Node::Penalty(7), Node::Penalty(9)]);
        let before = universe.page_material_counters();

        assert!(
            !universe
                .release_page_suffix_if_rootless(true)
                .expect("external root check is nonmutating")
        );
        assert!(universe.page_node_list(unreachable).is_ok());
        assert_eq!(universe.page_material_counters(), before);

        assert!(
            universe
                .release_page_suffix_if_rootless(false)
                .expect("fully rootless boundary releases")
        );
        assert!(universe.page_node_list(unreachable).is_err());
        assert!(
            universe
                .page_material_counters()
                .rootless_suffix_chunks_released
                > before.rootless_suffix_chunks_released
        );
    })
    .expect("universe allocation");
}

#[test]
fn page_checkpoint_fork_loans_one_timeline_and_rejection_restores_the_source_head() {
    with_universe(budget(), |universe| {
        universe
            .command_context()
            .expect("context")
            .append_page_contribution(Node::Penalty(1));
        let first = universe.runtime_checkpoint().expect("first page mark");
        universe
            .command_context()
            .expect("context")
            .append_page_contribution(Node::Penalty(2));
        let _head = universe.runtime_checkpoint().expect("source head mark");

        let mut candidate = universe
            .fork_runtime_checkpoint(&first)
            .expect("older page mark forks");
        let candidate_root = candidate.page_region.builder().contribution_root();
        assert_eq!(
            candidate
                .page_region
                .nodes()
                .node_cursor(candidate_root)
                .expect("candidate contribution root")
                .len(),
            1
        );
        let (mut nodes, page) = candidate.page_region.parts_mut();
        page.push_contribution(&mut nodes, Node::Penalty(3));
        universe.reject_checkpoint_candidate(&mut candidate);

        assert_eq!(
            universe
                .command_context()
                .expect("restored source context")
                .page_contributions()
                .iter()
                .cloned()
                .collect::<Vec<_>>(),
            [Node::Penalty(1), Node::Penalty(2)]
        );
    })
    .expect("universe allocation");
}

#[test]
fn nested_shipout_scratch_resets_suffixes_and_reuses_high_water() {
    with_universe(budget(), |universe| {
        let outer_id;
        let inner_id;
        {
            let mut outer = universe.begin_shipout();
            outer_id = outer.begin_shipout_scratch_list();
            for _ in 0..32 {
                outer.push_shipout_scratch_node(outer_id, Node::Penalty(1));
            }
            assert!(outer.shipout_scratch_nodes(outer_id).is_some());
            {
                let mut inner = outer.begin_shipout();
                inner_id = inner.begin_shipout_scratch_list();
                for _ in 0..64 {
                    inner.push_shipout_scratch_node(inner_id, Node::Penalty(2));
                }
                assert!(inner.shipout_scratch_nodes(inner_id).is_some());
            }
            assert!(outer.shipout_scratch_nodes(inner_id).is_none());
            assert!(outer.shipout_scratch_nodes(outer_id).is_some());
        }
        assert!(universe.shipout_scratch_nodes(outer_id).is_none());
        let warmed = universe.shipout_scratch_high_water();
        assert_eq!(warmed.0, 2);

        for _ in 0..256 {
            let mut transaction = universe.begin_shipout();
            let id = transaction.begin_shipout_scratch_list();
            for _ in 0..32 {
                transaction.push_shipout_scratch_node(id, Node::Penalty(3));
            }
            assert_eq!(
                transaction
                    .shipout_scratch_nodes(id)
                    .expect("scratch row is live")
                    .len(),
                32
            );
        }
        assert_eq!(universe.shipout_scratch_high_water(), warmed);
    })
    .expect("universe allocation");
}

#[test]
fn nested_shipout_engine_usage_accept_and_reject_restore_exact_membership() {
    with_universe(budget(), |universe| {
        {
            let mut context = universe.command_context().expect("context");
            context.slow_make_string_pool_string("retained");
        }
        let baseline = universe
            .command_context()
            .expect("baseline context")
            .detach_engine_usage_statistics();

        {
            let mut outer = universe.begin_shipout();
            outer
                .command_context()
                .expect("outer context")
                .slow_make_string_pool_string("outer-speculative");
            {
                let mut inner = outer.begin_shipout();
                inner
                    .command_context()
                    .expect("inner context")
                    .slow_make_string_pool_string("inner-accepted");
                inner.commit_for_test();
            }
            let nested = outer
                .command_context()
                .expect("nested context")
                .detach_engine_usage_statistics();
            assert_eq!(nested.strings, baseline.strings + 2);
        }

        let mut context = universe.command_context().expect("restored context");
        assert_eq!(context.detach_engine_usage_statistics(), baseline);
        context.slow_make_string_pool_string("outer-speculative");
        context.slow_make_string_pool_string("inner-accepted");
        assert_eq!(
            context.detach_engine_usage_statistics().strings,
            baseline.strings + 2,
            "outer rejection removes its own and nested accepted membership suffixes"
        );
        drop(context);

        let before_accept = universe
            .command_context()
            .expect("pre-accept context")
            .detach_engine_usage_statistics();
        {
            let mut accepted = universe.begin_shipout();
            accepted
                .command_context()
                .expect("accepted context")
                .slow_make_string_pool_string("top-level-accepted");
            accepted.commit_for_test();
        }
        let mut context = universe.command_context().expect("committed context");
        assert_eq!(
            context.detach_engine_usage_statistics().strings,
            before_accept.strings + 1
        );
        context.slow_make_string_pool_string("top-level-accepted");
        assert_eq!(
            context.detach_engine_usage_statistics().strings,
            before_accept.strings + 1,
            "accepted membership remains visible after the transaction mark settles"
        );
    })
    .expect("universe allocation");
}

#[test]
fn malformed_aggregate_restore_does_not_touch_dense_state() {
    with_universe(budget(), |universe| {
        let before_page = universe.page_node_cursor();
        let _ = universe.publish_page_nodes(&[Node::Penalty(7)]);
        let boundary = universe
            .page_region
            .nodes_mut()
            .seal_boundary()
            .expect("sealed page tail");
        let page = universe
            .page_region
            .nodes()
            .checkpoint_mark(boundary)
            .expect("page checkpoint");
        let malformed = universe
            .state_checkpoint_at(page)
            .expect("future page cursor");
        universe
            .assign_count(0, 41, AssignmentScope::Global)
            .expect("candidate count");
        universe
            .truncate_page_nodes(before_page)
            .expect("discard page suffix before restore");

        assert_eq!(
            universe.restore_state_checkpoint(&malformed),
            Err(UniverseError::State(StateError::InvalidCursor))
        );
        assert_eq!(
            universe
                .command_context()
                .expect("test fixture is valid")
                .count(0)
                .expect("test fixture is valid"),
            41,
            "page-cursor rejection must precede dense-state mutation"
        );
    })
    .expect("universe allocation");
}

#[test]
fn runtime_checkpoint_transfers_external_roots_before_suffix_truncation() {
    with_universe(budget(), |universe| {
        let checkpoint = universe.runtime_checkpoint().expect("runtime checkpoint");
        let suffix = universe.publish_page_nodes(&[Node::Penalty(99)]);
        let transferred = std::cell::Cell::new(false);
        universe
            .restore_runtime_checkpoint_with_roots(&checkpoint, || {
                transferred.set(true);
            })
            .expect("restore runtime checkpoint");
        assert!(transferred.get(), "external roots transferred");
        assert_eq!(
            universe
                .page_node_list(suffix)
                .expect_err("runtime suffix was truncated"),
            NodeArenaError::InvalidList
        );
    })
    .expect("universe allocation");
}

#[test]
fn runtime_checkpoint_restores_mutable_font_state() {
    with_universe(budget(), |universe| {
        let checkpoint = universe.runtime_checkpoint().expect("runtime checkpoint");
        {
            let mut context = universe.command_context().expect("context");
            context.set_font_hyphen_char(crate::font::NULL_FONT, 99);
            context
                .set_font_dimen(crate::font::NULL_FONT, 1, Scaled::from_raw(123))
                .expect("fontdimen");
        }
        universe
            .restore_runtime_checkpoint_with_roots(&checkpoint, || {})
            .expect("restore runtime checkpoint");
        let context = universe.command_context().expect("context");
        assert_eq!(
            context.font_hyphen_char(crate::font::NULL_FONT),
            i32::from(b'-')
        );
        assert_eq!(
            context.font_dimen(crate::font::NULL_FONT, 1),
            Scaled::from_raw(0)
        );
    })
    .expect("universe allocation");
}

#[test]
fn runtime_checkpoint_preserves_exact_font_roots_across_every_state_owner() {
    with_universe(budget(), |universe| {
        let selector = universe.intern("checkpointfont").expect("selector");
        let font = {
            let mut context = universe.command_context().expect("context");
            let font = context.intern_font(test_font("checkpointfont"));
            context
                .assign_resolved_meaning(
                    selector.symbol(),
                    ResolvedMeaning::Static(Meaning::Font(font)),
                    AssignmentScope::Global,
                )
                .expect("font meaning");
            context
                .assign_current_font(font, AssignmentScope::Global)
                .expect("current font");
            context.append_page_contribution(Node::Char {
                font,
                ch: 'A',
                origin: crate::token::OriginId::UNKNOWN,
            });
            context.set_pdf_font_attribute(font, b"checkpoint".to_vec());
            font
        };
        let checkpoint = universe.runtime_checkpoint().expect("runtime checkpoint");
        let suffix = universe
            .command_context()
            .expect("context")
            .intern_font(test_font("suffixfont"));
        universe
            .assign_current_font(suffix, AssignmentScope::Global)
            .expect("suffix font");

        universe
            .restore_runtime_checkpoint_with_roots(&checkpoint, || {})
            .expect("restore exact font roots");
        let context = universe.command_context().expect("context");
        assert_eq!(context.current_font(), font);
        assert_eq!(
            context.meaning(selector.symbol()),
            ResolvedMeaning::Static(Meaning::Font(font))
        );
        assert_eq!(context.font_name(font), "checkpointfont");
        assert!(matches!(
            context.page_contribution_front(),
            Some(Node::Char { font: retained, .. }) if *retained == font
        ));
    })
    .expect("universe allocation");
}

#[test]
fn checkpoint_capture_and_restore_do_not_scan_font_bearing_roots() {
    let large_budget = InternerBudget::new(512, 512, 65_536).expect("large font fixture budget");
    with_universe(large_budget, |universe| {
        let mut fonts = Vec::new();
        for index in 0..24 {
            let name = format!("checkpointfont{index:03}");
            let selector = universe.intern(&name).expect("font selector");
            let font = universe
                .command_context()
                .expect("context")
                .intern_font(test_font(&name));
            {
                let mut context = universe.command_context().expect("context");
                context
                    .assign_resolved_meaning(
                        selector.symbol(),
                        ResolvedMeaning::Static(Meaning::Font(font)),
                        AssignmentScope::Global,
                    )
                    .expect("font meaning");
                context.set_pdf_font_attribute(font, name.into_bytes());
            }
            fonts.push(font);
        }

        let first = fonts[0];
        let source_identity = universe.fonts.get(first).source_identity();
        let font_address = std::ptr::from_ref(universe.fonts.get(first));
        let mut children = universe.publish_page_nodes(&[Node::Char {
            font: first,
            ch: 'A',
            origin: crate::token::OriginId::UNKNOWN,
        }]);
        for depth in 0..24 {
            children = universe.publish_page_nodes(&[Node::HList(BoxNode::new(BoxNodeFields {
                width: Scaled::from_raw(depth),
                height: Scaled::from_raw(0),
                depth: Scaled::from_raw(0),
                shift: Scaled::from_raw(0),
                box_lr: BoxLr::Normal,
                glue_set: GlueSetRatio::ZERO,
                glue_sign: Sign::Normal,
                glue_order: crate::glue::Order::Normal,
                children,
            }))]);
        }
        universe
            .command_context()
            .expect("context")
            .append_page_contribution(Node::HList(BoxNode::new(BoxNodeFields {
                width: Scaled::from_raw(0),
                height: Scaled::from_raw(0),
                depth: Scaled::from_raw(0),
                shift: Scaled::from_raw(0),
                box_lr: BoxLr::Normal,
                glue_set: GlueSetRatio::ZERO,
                glue_sign: Sign::Normal,
                glue_order: crate::glue::Order::Normal,
                children,
            })));

        let checkpoint = universe.runtime_checkpoint().expect("font-rich checkpoint");
        assert_eq!(
            universe.runtime_checkpoint_font_scan_counters(),
            crate::RuntimeCheckpointFontScanCounters::default()
        );
        let stale = universe
            .command_context()
            .expect("context")
            .intern_font(test_font("discarded-font"));
        universe
            .restore_runtime_checkpoint_with_roots(&checkpoint, || {})
            .expect("same-generation restore");

        assert_eq!(
            universe.runtime_checkpoint_font_scan_counters(),
            crate::RuntimeCheckpointFontScanCounters::default()
        );
        assert_eq!(std::ptr::from_ref(universe.fonts.get(first)), font_address);
        assert!(!universe.fonts.contains(stale));
        assert_eq!(
            universe
                .command_context()
                .expect("context")
                .font_id_for_source_identity(source_identity),
            Some(first),
            "generated-source lookup remains available from the retained prefix"
        );
    })
    .expect("universe allocation");
}

#[test]
fn stale_font_root_is_rejected_at_the_publication_seam() {
    with_universe(budget(), |universe| {
        let checkpoint = universe.runtime_checkpoint().expect("checkpoint");
        let stale = universe
            .command_context()
            .expect("context")
            .intern_font(test_font("stale-font"));
        universe
            .restore_runtime_checkpoint_with_roots(&checkpoint, || {})
            .expect("discard font suffix");
        let selector = universe.intern("stale").expect("selector");

        assert_eq!(
            universe.assign_current_font(stale, AssignmentScope::Global),
            Err(UniverseError::State(StateError::ForeignSession))
        );
        assert_eq!(
            universe.assign_meaning(
                selector,
                MeaningWord::from_static(Meaning::Font(stale)),
                AssignmentScope::Global,
            ),
            Err(UniverseError::State(StateError::ForeignSession))
        );
        universe
            .runtime_checkpoint()
            .expect("rejected stale roots never enter a checkpoint");
    })
    .expect("universe allocation");
}

#[test]
fn boundary_hash_includes_mutable_font_runtime() {
    with_universe(budget(), |universe| {
        let before = universe.engine_boundary_hash(23, |hash| hash.font(crate::font::NULL_FONT));
        {
            let mut context = universe.command_context().expect("context");
            context.set_font_skew_char(crate::font::NULL_FONT, 17);
        }
        let after = universe.engine_boundary_hash(23, |hash| hash.font(crate::font::NULL_FONT));
        assert_ne!(before, after);
    })
    .expect("universe allocation");
}

#[test]
fn admitted_paragraph_shape_is_detached_and_group_restorable() {
    with_universe(budget(), |universe| {
        let mut context = universe.command_context().expect("context");
        let baseline = [ParagraphShapeLine {
            indent: Scaled::from_raw(10),
            width: Scaled::from_raw(100),
        }];
        context
            .assign_paragraph_shape(&baseline, AssignmentScope::Global)
            .expect("baseline shape");
        context.begin_group(GroupKind::Simple, 1).expect("group");
        let local = [
            ParagraphShapeLine {
                indent: Scaled::from_raw(20),
                width: Scaled::from_raw(200),
            },
            ParagraphShapeLine {
                indent: Scaled::from_raw(30),
                width: Scaled::from_raw(300),
            },
        ];
        context
            .assign_paragraph_shape(&local, AssignmentScope::Local)
            .expect("local shape");

        assert_eq!(context.paragraph_shape(), local);
        assert_eq!(context.paragraph_shape_len(), 2);
        assert_eq!(
            context.paragraph_shape_dimension(3, false),
            Scaled::from_raw(30),
            "lines after the explicit shape repeat its final entry"
        );
        assert_eq!(
            context.paragraph_shape_dimension(3, true),
            Scaled::from_raw(300)
        );
        assert_eq!(
            context.paragraph_shape_dimension(0, true),
            Scaled::from_raw(0)
        );

        context.end_group(GroupKind::Simple).expect("end group");
        assert_eq!(context.paragraph_shape(), baseline);
    })
    .expect("universe allocation");
}

#[test]
fn admitted_penalty_arrays_preserve_etex_projection_and_scope() {
    with_universe(budget(), |universe| {
        let mut context = universe.command_context().expect("context");
        context.begin_group(GroupKind::Simple, 1).expect("group");
        context
            .assign_penalty_array(
                PenaltyArrayKind::Club,
                &[10, 20, 30],
                AssignmentScope::Local,
            )
            .expect("local penalty array");

        assert_eq!(context.penalty_array(PenaltyArrayKind::Club), [10, 20, 30]);
        assert_eq!(context.penalty_array_value(PenaltyArrayKind::Club, -1), 0);
        assert_eq!(context.penalty_array_value(PenaltyArrayKind::Club, 0), 3);
        assert_eq!(context.penalty_array_value(PenaltyArrayKind::Club, 2), 20);
        assert_eq!(context.penalty_array_value(PenaltyArrayKind::Club, 8), 30);

        context.end_group(GroupKind::Simple).expect("end group");
        assert!(context.penalty_array(PenaltyArrayKind::Club).is_empty());
        assert_eq!(context.penalty_array_value(PenaltyArrayKind::Club, 0), 0);
    })
    .expect("universe allocation");
}

#[test]
fn admitted_assignment_rendering_never_reopens_the_universe() {
    with_universe(budget(), |universe| {
        let symbol = universe.intern("alpha").expect("intern");
        universe
            .assign_meaning(
                symbol,
                MeaningWord::from_static(Meaning::Relax),
                AssignmentScope::Global,
            )
            .expect("meaning");
        let mut context = universe.command_context().expect("context");
        assert_eq!(
            context.bounded_meaning_text(Token::Cs(symbol.symbol()), 32),
            "\\relax"
        );
        assert_eq!(context.box_assignment_trace_text(None), "void");

        let children = context.publish_page_nodes(Vec::new());
        let root = context.publish_page_nodes(vec![Node::HList(BoxNode::new(BoxNodeFields {
            width: Scaled::from_raw(0),
            height: Scaled::from_raw(0),
            depth: Scaled::from_raw(0),
            shift: Scaled::from_raw(0),
            box_lr: BoxLr::Normal,
            glue_set: GlueSetRatio::ZERO,
            glue_sign: Sign::Normal,
            glue_order: crate::glue::Order::Normal,
            children,
        }))]);
        assert_eq!(
            context.box_assignment_trace_text(Some(root)),
            "\\hbox(0.0+0.0)x0.0"
        );

        let children = context.publish_page_nodes(vec![Node::Penalty(0)]);
        let root = context.publish_page_nodes(vec![Node::HList(BoxNode::new(BoxNodeFields {
            width: Scaled::from_raw(0),
            height: Scaled::from_raw(0),
            depth: Scaled::from_raw(0),
            shift: Scaled::from_raw(0),
            box_lr: BoxLr::Normal,
            glue_set: GlueSetRatio::ZERO,
            glue_sign: Sign::Normal,
            glue_order: crate::glue::Order::Normal,
            children,
        }))]);
        assert_eq!(
            context.box_assignment_trace_text(Some(root)),
            "\\hbox(0.0+0.0)x0.0 []"
        );

        let children = context.publish_page_nodes(vec![Node::Penalty(0)]);
        let root = context.publish_page_nodes(vec![Node::VList(BoxNode::new(BoxNodeFields {
            width: Scaled::from_raw(0),
            height: Scaled::from_raw(0),
            depth: Scaled::from_raw(0),
            shift: Scaled::from_raw(65_536),
            box_lr: BoxLr::Reversed,
            glue_set: GlueSetRatio::from_ratio_parts(1, 2),
            glue_sign: Sign::Shrinking,
            glue_order: crate::glue::Order::Fill,
            children,
        }))]);
        assert_eq!(
            context.box_assignment_trace_text(Some(root)),
            "\\vbox(0.0+0.0)x0.0, glue set - 0.5fill, shifted 1.0, reversed []"
        );
    })
    .expect("universe allocation");
}

#[test]
fn dropped_shipout_restores_aggregate_roots_before_page_suffix_truncation() {
    with_universe(budget(), |universe| {
        universe
            .assign_count(0, 7, AssignmentScope::Global)
            .expect("baseline count");
        let retained_root = universe.publish_page_nodes(&[Node::Penalty(3)]);
        universe
            .command_context()
            .expect("context")
            .append_page_contribution(Node::HList(BoxNode::new(BoxNodeFields {
                width: Scaled::from_raw(0),
                height: Scaled::from_raw(0),
                depth: Scaled::from_raw(0),
                shift: Scaled::from_raw(0),
                box_lr: BoxLr::Normal,
                glue_set: GlueSetRatio::ZERO,
                glue_sign: Sign::Normal,
                glue_order: crate::glue::Order::Normal,
                children: retained_root,
            })));
        let failed_region = universe.begin_page_node_region();
        let failed_operand = universe.publish_page_nodes(&[Node::Penalty(11)]);
        let speculative_root = {
            let mut transaction = universe.begin_shipout();
            transaction
                .assign_count(0, 99, AssignmentScope::Global)
                .expect("speculative count");
            transaction
                .world_mut()
                .write_text(crate::PrintSink::Terminal, "speculative");
            let mut context = transaction.command_context().expect("context");
            let children = context.publish_page_nodes(vec![Node::Penalty(17)]);
            context.append_page_contribution(Node::HList(BoxNode::new(BoxNodeFields {
                width: Scaled::from_raw(0),
                height: Scaled::from_raw(0),
                depth: Scaled::from_raw(0),
                shift: Scaled::from_raw(0),
                box_lr: BoxLr::Normal,
                glue_set: GlueSetRatio::ZERO,
                glue_sign: Sign::Normal,
                glue_order: crate::glue::Order::Normal,
                children,
            })));
            children
        };

        assert_eq!(universe.count(0).expect("count"), 7);
        assert!(universe.page_node_list(speculative_root).is_err());
        assert!(universe.page_node_list(failed_operand).is_ok());
        assert!(universe.world().effect_records().is_empty());
        assert_eq!(
            universe
                .command_context()
                .expect("context")
                .page_contributions()
                .len(),
            1
        );

        universe
            .release_page_node_region(failed_region)
            .expect("failed shipout releases only its operand region");
        assert!(universe.page_node_list(failed_operand).is_err());
        assert!(universe.page_node_list(retained_root).is_ok());
        assert_eq!(
            universe
                .command_context()
                .expect("context")
                .page_contributions()
                .len(),
            1
        );
    })
    .expect("universe allocation");
}

#[test]
fn pure_memo_capability_is_borrowed_and_does_not_keep_runtime_alive() {
    with_universe(budget(), |universe| {
        let runtime = std::sync::Arc::new(std::sync::Mutex::new(crate::PureMemoRuntime::default()));
        universe.attach_pure_memo_capability(&runtime);
        assert!(
            universe
                .with_pure_memo(|_| 41)
                .is_some_and(|value| value == 41)
        );
        drop(runtime);
        assert_eq!(universe.with_pure_memo(|_| 0), None);
    })
    .expect("universe allocation");
}

#[test]
fn engine_boundary_hash_resolves_children_and_erases_runtime_provenance() {
    with_universe(budget(), |universe| {
        let left = universe.publish_page_nodes(&[Node::Penalty(7)]);
        let right = universe.publish_page_nodes(&[Node::Penalty(7)]);
        let boxed = |children| {
            Node::HList(BoxNode::new(BoxNodeFields {
                width: Scaled::from_raw(10),
                height: Scaled::from_raw(20),
                depth: Scaled::from_raw(3),
                shift: Scaled::from_raw(0),
                box_lr: BoxLr::Normal,
                glue_set: GlueSetRatio::ZERO,
                glue_sign: Sign::Normal,
                glue_order: crate::glue::Order::Normal,
                children,
            }))
        };
        let left_hash = universe.engine_boundary_hash(17, |hash| hash.nodes(&[boxed(left)]));
        let right_hash = universe.engine_boundary_hash(17, |hash| hash.nodes(&[boxed(right)]));
        assert_eq!(left_hash, right_hash, "page row identity is nonsemantic");

        let character = |origin| Node::Char {
            font: crate::font::NULL_FONT,
            ch: 'x',
            origin,
        };
        let unknown = universe.engine_boundary_hash(19, |hash| {
            hash.nodes(&[character(crate::token::OriginId::UNKNOWN)]);
        });
        let sourced = universe.engine_boundary_hash(19, |hash| {
            hash.nodes(&[character(crate::token::OriginId::from_raw(41))]);
        });
        assert_eq!(unknown, sourced, "diagnostic provenance is nonsemantic");
        let changed = universe.engine_boundary_hash(19, |hash| {
            hash.nodes(&[Node::Char {
                font: crate::font::NULL_FONT,
                ch: 'y',
                origin: crate::token::OriginId::UNKNOWN,
            }]);
        });
        assert_ne!(unknown, changed);
    })
    .expect("universe allocation");
}

#[test]
fn multi_byte_source_origin_detaches_the_complete_registered_range() {
    with_universe(budget(), |universe| {
        let source = crate::input::SourceId::new(41);
        let bytes: std::sync::Arc<[u8]> = std::sync::Arc::from(&b"x\\input y"[..]);
        let mut context = universe.command_context().expect("command admission");
        context
            .register_source(
                source,
                crate::source_map::SourceDescriptor::named_generated("generated/main.tex", bytes),
            )
            .expect("source registration");

        let origin = context.source_range_origin(source, 1, 7);
        let detached = context
            .detach_diagnostic_origin(
                origin,
                crate::DiagnosticOriginRequest {
                    demand: crate::ColdProvenanceDemand::Diagnostic,
                    message: "failure",
                },
            )
            .expect("diagnostic detachment");

        let resolved = detached.resolved_source.expect("resolved source");
        assert_eq!((resolved.start, resolved.end), (1, 7));
        assert_eq!(resolved.excerpt, "x\\input y");
        let generated = detached.generated_origin.expect("generated source recipe");
        assert_eq!((generated.start, generated.end), (1, 7));
    })
    .expect("universe allocation");
}

#[test]
fn page_node_transform_counts_new_payload_and_never_copies_source_nodes() {
    with_universe(budget(), |universe| {
        let mut context = universe.command_context().expect("command admission");
        let left = context.publish_page_node_range(vec![Node::Penalty(1), Node::Penalty(2)]);
        let right = context.publish_page_node_range(vec![Node::Penalty(3), Node::Penalty(4)]);
        let source = context.compose_page_node_sequences(&[left, right]);
        let mut scratch = crate::node_arena::PageNodeTransformScratch::default();
        context.begin_page_node_transform(&mut scratch);
        context.retain_page_node_source_range(&mut scratch, source, 0..1);
        context.append_new_page_nodes(&mut scratch, vec![Node::Penalty(9)]);
        context.retain_page_node_source_range(&mut scratch, source, 3..4);
        let transformed = context.finish_page_node_transform(&mut scratch);

        assert_eq!(scratch.new_semantic_nodes(), 1);
        assert_eq!(scratch.source_nodes_copied(), 0);
        assert_eq!(
            context
                .page_node_sequence(transformed)
                .expect("transformed sequence resolves")
                .iter()
                .cloned()
                .collect::<Vec<_>>(),
            [Node::Penalty(1), Node::Penalty(9), Node::Penalty(4)]
        );

        context.begin_page_node_transform(&mut scratch);
        assert_eq!(scratch.new_semantic_nodes(), 0);
        assert_eq!(scratch.source_nodes_copied(), 0);
    })
    .expect("universe allocation");
}
