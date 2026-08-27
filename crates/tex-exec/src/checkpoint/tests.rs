use tex_command::{
    CommandHostCapabilities, CommandHostContext, CommandProcessor, CommandProfile,
    CommandRestoreError, CommandState,
};
use tex_state::env::AssignmentScope;
use tex_state::meaning::{Meaning, ResolvedMeaning};
use tex_state::token::{Catcode, Token, TokenWord};

use super::{
    CheckpointOwnerFamily, CheckpointRestoreError, EngineBoundary, EngineCheckpoint,
    ReachableStateRoots,
};
use crate::{
    AlignColumn, AlignState, AlignmentKind, AlignmentPackSpec, ExecutionBudgetCounters, Mode,
    ModeNest,
};

#[test]
fn ordinary_and_requested_capture_never_traverse_mode_payload_for_identity() {
    crate::test_harness::with_nonstop_universe(|universe| {
        let mut command = CommandState::default();
        let mut modes = ModeNest::new();
        modes
            .current_list_mutation()
            .push(tex_state::node::Node::Penalty(17));
        crate::mode::reset_semantic_fingerprint_calls_for_test();

        let ordinary = EngineCheckpoint::capture_checkpoint(
            EngineBoundary::JobStart,
            &mut command,
            &mut modes,
            universe,
            ExecutionBudgetCounters::default(),
        )
        .expect("ordinary checkpoint");
        assert_eq!(ordinary.reachable_state_identity(), None);
        assert_eq!(crate::mode::semantic_fingerprint_calls_for_test(), 0);

        let requested = EngineCheckpoint::capture_checkpoint_with_identity_demand(
            EngineBoundary::OuterParagraphEnd,
            &mut command,
            &mut modes,
            universe,
            ExecutionBudgetCounters::default(),
            true,
        )
        .expect("requested checkpoint");
        assert_eq!(requested.reachable_state_identity(), None);
        assert_eq!(
            crate::mode::semantic_fingerprint_calls_for_test(),
            0,
            "missing component roots fail closed without a payload traversal"
        );
    });
}

#[test]
fn complete_identity_is_versioned_and_every_component_is_semantic() {
    let roots = ReachableStateRoots {
        command: Some(1),
        mode: Some(2),
        page: Some(3),
        world: Some(4),
        hyphenation: Some(5),
        pdf: Some(6),
        dependency: Some(7),
        source: Some(8),
        font: Some(9),
        core: Some(10),
    };
    let baseline = roots.complete().expect("all component roots are present");
    assert_eq!(
        baseline.schema_version(),
        super::REACHABLE_STATE_IDENTITY_SCHEMA_VERSION
    );
    for component in 0..10 {
        let mut changed = roots;
        let root = match component {
            0 => &mut changed.command,
            1 => &mut changed.mode,
            2 => &mut changed.page,
            3 => &mut changed.world,
            4 => &mut changed.hyphenation,
            5 => &mut changed.pdf,
            6 => &mut changed.dependency,
            7 => &mut changed.source,
            8 => &mut changed.font,
            9 => &mut changed.core,
            _ => unreachable!(),
        };
        *root = Some(root.expect("root exists") ^ 0x8000_0000_0000_0000);
        assert_ne!(
            changed.complete(),
            Some(baseline),
            "component {component} must perturb complete identity"
        );
    }
    assert_eq!(
        ReachableStateRoots { pdf: None, ..roots }.complete(),
        None,
        "one missing root prevents a partial identity"
    );
}

#[test]
fn retention_descriptor_covers_every_aggregate_owner_family() {
    crate::test_harness::with_nonstop_universe(|universe| {
        let mut command = CommandState::default();
        let mut modes = ModeNest::new();
        let checkpoint = EngineCheckpoint::capture_checkpoint(
            EngineBoundary::JobStart,
            &mut command,
            &mut modes,
            universe,
            ExecutionBudgetCounters::default(),
        )
        .expect("checkpoint");
        let retention = checkpoint.retention();
        assert!(
            retention.command_bytes() > std::mem::size_of_val(&checkpoint.command),
            "command charge must name its owner storage, not its summary handle"
        );
        assert!(
            retention.mode_bytes() > std::mem::size_of_val(&checkpoint.modes),
            "mode charge must name its owner storage, not its checkpoint handle"
        );
        for (component, bytes) in [
            ("command", retention.command_bytes()),
            ("mode", retention.mode_bytes()),
            ("page", retention.page_bytes()),
            ("World", retention.world_bytes()),
            ("hyphenation", retention.hyphenation_bytes()),
            ("PDF", retention.pdf_bytes()),
            ("dependency", retention.dependency_bytes()),
            ("source/font", retention.source_font_bytes()),
            ("core", retention.core_bytes()),
            ("counter", retention.execution_counter_bytes()),
        ] {
            assert!(bytes > 0, "{component} retention charge is absent");
        }
        assert!(
            retention.checkpoint_metadata_bytes() >= retention.execution_counter_bytes(),
            "fixed checkpoint metadata includes its execution counters"
        );

        let later = EngineCheckpoint::capture_checkpoint(
            EngineBoundary::OuterParagraphEnd,
            &mut command,
            &mut modes,
            universe,
            ExecutionBudgetCounters::default(),
        )
        .expect("later checkpoint");
        let owner = |retention: super::CheckpointRetention, family| {
            retention
                .shared_owners()
                .iter()
                .find(|charge| charge.family() == family)
                .expect("every family publishes one charge")
                .owner()
        };
        for family in [
            CheckpointOwnerFamily::Command,
            CheckpointOwnerFamily::Mode,
            CheckpointOwnerFamily::Page,
            CheckpointOwnerFamily::World,
            CheckpointOwnerFamily::Hyphenation,
            CheckpointOwnerFamily::Pdf,
            CheckpointOwnerFamily::Dependency,
            CheckpointOwnerFamily::SourceFont,
        ] {
            assert_eq!(
                owner(retention, family),
                owner(later.retention(), family),
                "shared {family:?} owner must keep one accounting identity"
            );
        }
        assert_eq!(
            owner(retention, CheckpointOwnerFamily::Core),
            owner(later.retention(), CheckpointOwnerFamily::Core),
            "fixed core marks share the one accepted lineage owner"
        );
    });
}

#[test]
fn retained_checkpoint_restores_command_and_mode_token_roots() {
    crate::test_harness::with_nonstop_universe(|universe| {
        let command_root = universe
            .command_context()
            .expect("command context")
            .allocate_token_list(&[TokenWord::pack(Token::Char {
                ch: 'c',
                cat: Catcode::Other,
            })])
            .expect("command root");
        let u_template = tex_state::node::NodeTokenList::new([TokenWord::pack(Token::Char {
            ch: 'u',
            cat: Catcode::Other,
        })]);
        let v_template = tex_state::node::NodeTokenList::new([TokenWord::pack(Token::Char {
            ch: 'v',
            cat: Catcode::Other,
        })]);
        let mut command = CommandState::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        command.push_everypar(
            &universe.command_context().expect("command context"),
            command_root,
        );
        let pushes = command.publish_named_token_list_pushes(
            &mut universe.command_context().expect("command context"),
            &mut diagnostic_effects,
        );
        assert_eq!(pushes.len(), 1, "everypar publishes one retained push");
        let mut modes = ModeNest::new();
        modes
            .current_list_mutation()
            .set_align_state(AlignState::new(
                AlignmentKind::HAlign,
                AlignmentPackSpec::Natural,
                vec![AlignColumn {
                    u_template,
                    v_template,
                }],
                Vec::new(),
                tex_state::glue::GlueSpec::ZERO,
                None,
            ));
        let checkpoint = EngineCheckpoint::capture_checkpoint(
            EngineBoundary::JobStart,
            &mut command,
            &mut modes,
            universe,
            ExecutionBudgetCounters::default(),
        )
        .expect("checkpoint captures");

        let later_command_root = universe
            .command_context()
            .expect("command context")
            .allocate_token_list(&[TokenWord::pack(Token::Char {
                ch: 'x',
                cat: Catcode::Other,
            })])
            .expect("later command root");
        command.push_everypar(
            &universe.command_context().expect("command context"),
            later_command_root,
        );
        let _ = command.publish_named_token_list_pushes(
            &mut universe.command_context().expect("command context"),
            &mut diagnostic_effects,
        );
        modes
            .current_list_mutation()
            .take_align_state()
            .expect("alignment root is mutated after capture");
        checkpoint
            .restore_state(&mut command, &mut modes, universe)
            .expect("retained checkpoint restores into its owning timeline");

        let mut capabilities = CommandHostCapabilities::default();
        let mut fuel = tex_command::CommandFuelLedger::default();
        let mut context = universe.command_context().expect("command context");
        let mut processor = CommandProcessor::new(
            &mut command,
            &mut context,
            CommandHostContext::new(&mut capabilities),
            fuel.fuel_mut(),
            None,
            &mut diagnostic_effects,
        );
        assert_eq!(
            processor
                .get_next()
                .expect("restored command delivery")
                .expect("restored command token")
                .meaning(),
            ResolvedMeaning::Static(Meaning::CharToken {
                ch: 'c',
                cat: Catcode::Other,
            })
        );
        drop(processor);

        let current_list = modes.current_list();
        let column = &current_list
            .align_state()
            .expect("restored alignment")
            .columns()[0];
        assert_eq!(
            column.u_template.words(),
            &[TokenWord::pack(Token::Char {
                ch: 'u',
                cat: Catcode::Other,
            })]
        );
        assert_eq!(
            column.v_template.words(),
            &[TokenWord::pack(Token::Char {
                ch: 'v',
                cat: Catcode::Other,
            })]
        );
    });
}

#[test]
fn retained_checkpoint_rejects_a_fresh_command_timeline_before_mutation() {
    crate::test_harness::with_nonstop_universe(|universe| {
        universe
            .assign_count(0, 10, AssignmentScope::Global)
            .expect("baseline count");
        let checkpoint = EngineCheckpoint::capture_checkpoint(
            EngineBoundary::JobStart,
            &mut CommandState::default(),
            &mut ModeNest::new(),
            universe,
            ExecutionBudgetCounters::default(),
        )
        .expect("checkpoint captures");
        universe
            .assign_count(0, 20, AssignmentScope::Global)
            .expect("candidate count");
        let mut command = CommandState::default();
        let mut modes = ModeNest::new();
        let mode_fingerprint_before = modes.summary().semantic_fingerprint(universe);

        assert!(matches!(
            checkpoint.restore_state(&mut command, &mut modes, universe),
            Err(CheckpointRestoreError::Command(
                CommandRestoreError::ForeignGeneration
            ))
        ));
        assert_eq!(
            modes.summary().semantic_fingerprint(universe),
            mode_fingerprint_before,
            "foreign-timeline validation must precede mode mutation"
        );
        assert_eq!(
            universe
                .command_context()
                .expect("command context")
                .count(0)
                .expect("count"),
            20,
            "foreign-timeline validation must precede runtime mutation"
        );
    });
}

#[test]
fn command_validation_failure_leaves_runtime_and_mode_unchanged() {
    crate::test_harness::with_nonstop_universe(|universe| {
        universe
            .assign_count(0, 10, AssignmentScope::Global)
            .expect("baseline count");
        let checkpoint = EngineCheckpoint::capture_checkpoint(
            EngineBoundary::JobStart,
            &mut CommandState::new(CommandProfile::TEX82),
            &mut ModeNest::new(),
            universe,
            ExecutionBudgetCounters::default(),
        )
        .expect("checkpoint captures");
        universe
            .assign_count(0, 20, AssignmentScope::Global)
            .expect("candidate count");
        let mut command = CommandState::new(CommandProfile::PDFTEX14029);
        let mut modes = ModeNest::new();

        assert!(matches!(
            checkpoint.restore_state(&mut command, &mut modes, universe),
            Err(CheckpointRestoreError::Command(_))
        ));
        assert_eq!(command.profile(), CommandProfile::PDFTEX14029);
        assert_eq!(
            universe
                .command_context()
                .expect("command context")
                .count(0)
                .expect("count"),
            20,
            "command validation must precede runtime mutation"
        );
    });
}

#[test]
fn checkpoint_restore_does_not_refund_nest_high_water() {
    // TeX82 §§216/1334: semantic mode roots roll back, but a job-lifetime
    // maximum already observed by `push_nest` does not.
    crate::test_harness::with_nonstop_universe(|universe| {
        let mut command = CommandState::default();
        let mut modes = ModeNest::new();
        modes.push(Mode::Horizontal).expect("horizontal mode");
        modes.push(Mode::Math).expect("math mode");
        let checkpoint = EngineCheckpoint::capture_checkpoint(
            EngineBoundary::OuterParagraphEnd,
            &mut command,
            &mut modes,
            universe,
            ExecutionBudgetCounters::default(),
        )
        .expect("checkpoint captures");

        modes
            .push(Mode::RestrictedHorizontal)
            .expect("later nested mode");
        assert_eq!(modes.maximum_saved_depth(), 2);
        checkpoint
            .restore_state(&mut command, &mut modes, universe)
            .expect("checkpoint restores");
        assert_eq!(modes.depth(), 3, "semantic mode summary rolled back");
        assert_eq!(
            modes.maximum_saved_depth(),
            2,
            "operational high-water survives rollback"
        );
    });
}

#[test]
fn rejected_mode_fork_returns_the_coarse_timeline_without_branch_nodes() {
    crate::test_harness::with_nonstop_universe(|universe| {
        let mut command = CommandState::default();
        let mut modes = ModeNest::new();
        modes.push_current_node(tex_state::node::Node::Penalty(11));
        let checkpoint = EngineCheckpoint::capture_checkpoint(
            EngineBoundary::OuterParagraphEnd,
            &mut command,
            &mut modes,
            universe,
            ExecutionBudgetCounters::default(),
        )
        .expect("checkpoint captures");
        drop(command);

        let (mut rejected, mut branch, _ledger) = checkpoint
            .fork_state(universe)
            .expect("mode timeline forks");
        branch
            .mode_nest_mut_for_test()
            .push_current_node(tex_state::node::Node::Penalty(22));
        universe.reject_checkpoint_candidate(&mut rejected);
        drop(branch);
        drop(rejected);

        let (mut retried, retry, _ledger) = checkpoint
            .fork_state(universe)
            .expect("returned mode timeline forks again");
        assert_eq!(
            retry.mode_nest_for_test().current_list().nodes(),
            &[tex_state::node::Node::Penalty(11)],
            "candidate-only nodes must not escape the rejected coarse owner"
        );
        universe.reject_checkpoint_candidate(&mut retried);
        drop(retry);
    });
}

#[test]
fn early_rootless_fork_rejects_without_losing_the_large_accepted_head() {
    crate::test_harness::with_nonstop_universe(|universe| {
        let mut command = CommandState::default();
        let mut modes = ModeNest::new();
        let checkpoint = EngineCheckpoint::capture_checkpoint(
            EngineBoundary::JobStart,
            &mut command,
            &mut modes,
            universe,
            ExecutionBudgetCounters::default(),
        )
        .expect("early checkpoint captures");

        for index in 0..512 {
            modes.push_current_node(tex_state::node::Node::Penalty(index));
            universe
                .command_context()
                .expect("accepted context")
                .append_page_contribution(tex_state::node::Node::Penalty(index));
        }
        // The command timeline returns its exclusive roots before an aggregate
        // candidate may borrow them.
        drop(command);

        let (mut rejected, mut branch, _ledger) = checkpoint
            .fork_state(universe)
            .expect("early checkpoint forks");
        assert!(
            branch
                .mode_nest_for_test()
                .current_list()
                .nodes()
                .is_empty()
        );
        branch
            .mode_nest_mut_for_test()
            .push_current_node(tex_state::node::Node::Penalty(900));
        universe.reject_checkpoint_candidate(&mut rejected);
        drop(branch);
        drop(rejected);

        assert_eq!(
            universe
                .command_context()
                .expect("restored source")
                .page_contributions()
                .len(),
            512,
            "rejection returns the untouched accepted page payload",
        );
        let (mut retried, retry, _ledger) = checkpoint
            .fork_state(universe)
            .expect("returned early checkpoint forks again");
        assert!(retry.mode_nest_for_test().current_list().nodes().is_empty());
        universe.reject_checkpoint_candidate(&mut retried);
        drop(retry);
        drop(retried);
    });
}
