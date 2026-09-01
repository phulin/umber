use tex_command::{
    CommandHostCapabilities, CommandHostContext, CommandObservation, CommandObserver,
    CommandProcessor, CommandProfile, CommandRestoreError, CommandState,
};
use tex_state::env::AssignmentScope;
use tex_state::interner::InternerBudget;
use tex_state::meaning::{Meaning, ResolvedMeaning};
use tex_state::token::{Catcode, Token, TokenWord};
use tex_state::{ReachabilityStore, World};

use super::{
    CheckpointEligibility, CheckpointOwnerFamily, CheckpointRestoreError, EngineBoundary,
    EngineCheckpoint, ReachableStateRoots,
};
use crate::{
    AdmittedEngineGeneration, ExecutionBudgetCounters, Mode, ModeNest, RestoredCheckpointRuntime,
    RetainedCheckpointKey, RetainedEngineAttachmentKey, RetainedEngineGeneration,
    RetainedEngineOperation,
};

fn retained_store() -> ReachabilityStore {
    ReachabilityStore::new(
        InternerBudget::new(65_536, 131_072, 16 * 1024 * 1024)
            .expect("checkpoint test interner budget"),
    )
}

#[derive(Default)]
struct ObservationRecorder(Vec<CommandObservation>);

impl CommandObserver for ObservationRecorder {
    fn committed(&mut self, observation: CommandObservation) {
        self.0.push(observation);
    }
}

struct CaptureModeCheckpoint {
    accepted_tail_len: usize,
}

impl RetainedEngineOperation for CaptureModeCheckpoint {
    type Output = RetainedCheckpointKey;

    fn run<G: 'static>(self, mut admitted: AdmittedEngineGeneration<'_, G>) -> Self::Output {
        let mut control = crate::MainControl::tex82_initex(admitted.universe());
        let checkpoint = control
            .capture_checkpoint(
                EngineBoundary::OuterParagraphEnd,
                admitted.universe(),
                ExecutionBudgetCounters::default(),
            )
            .expect("checkpoint captures");
        for index in 0..self.accepted_tail_len {
            let penalty = i32::try_from(index).expect("test penalty fits");
            let mut context = admitted
                .universe()
                .command_context()
                .expect("accepted context");
            control
                .mode_nest_mut_for_test()
                .push_current_node(&mut context, tex_state::node::Node::Penalty(penalty));
            context.append_page_contribution(tex_state::node::Node::Penalty(penalty));
        }
        admitted
            .prepare_checkpoint_control(control)
            .expect("accepted command owner parks")
            .accept();
        admitted.retain_checkpoint(checkpoint)
    }
}

struct InspectAndRejectModeFork {
    runtime: RetainedEngineAttachmentKey,
    append_penalty: Option<i32>,
}

impl RetainedEngineOperation for InspectAndRejectModeFork {
    type Output = Vec<i32>;

    fn run<G: 'static>(self, mut admitted: AdmittedEngineGeneration<'_, G>) -> Self::Output {
        let _runtime = admitted
            .take_attachment::<RestoredCheckpointRuntime>(self.runtime)
            .expect("fork owns restored runtime");
        let mut control = admitted
            .take_checkpoint_control()
            .expect("fork owns typed main control");
        if let Some(penalty) = self.append_penalty {
            let mut context = admitted
                .universe()
                .command_context()
                .expect("candidate context");
            control
                .mode_nest_mut_for_test()
                .push_current_node(&mut context, tex_state::node::Node::Penalty(penalty));
        }
        let penalties = {
            let context = admitted
                .universe()
                .command_context()
                .expect("candidate inspection context");
            control
                .mode_nest_for_test()
                .current_list()
                .nodes(&context)
                .iter()
                .filter_map(|node| match node {
                    tex_state::NodeView::Penalty(penalty) => Some(penalty),
                    _ => None,
                })
                .collect()
        };
        admitted
            .prepare_checkpoint_control(control)
            .expect("rejected control parks")
            .reject();
        penalties
    }
}

struct PageContributionCount;

impl RetainedEngineOperation for PageContributionCount {
    type Output = usize;

    fn run<G: 'static>(self, mut admitted: AdmittedEngineGeneration<'_, G>) -> Self::Output {
        admitted
            .universe()
            .command_context()
            .expect("page inspection context")
            .page_contributions()
            .len()
    }
}

#[test]
fn ordinary_and_requested_capture_never_traverse_mode_payload_for_identity() {
    crate::test_harness::with_nonstop_universe(|universe| {
        let mut command = CommandState::default();
        let mut modes = ModeNest::new();
        modes.current_list_mutation().set_prev_graf(17);
        crate::mode::reset_semantic_fingerprint_calls_for_test();

        let ordinary = EngineCheckpoint::capture_checkpoint(
            CheckpointEligibility::job_start(),
            &mut command,
            &mut modes,
            universe,
            ExecutionBudgetCounters::default(),
        )
        .expect("ordinary checkpoint");
        assert_eq!(ordinary.reachable_state_identity(), None);
        assert_eq!(crate::mode::semantic_fingerprint_calls_for_test(), 0);

        let requested = EngineCheckpoint::capture_checkpoint_with_identity_demand(
            CheckpointEligibility::named(EngineBoundary::OuterParagraphEnd),
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
            CheckpointEligibility::job_start(),
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
        assert_eq!(
            retention.mode_bytes(),
            checkpoint.modes.retained_owner_bytes(),
            "mode retention is exactly one scalar outer-level summary"
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
            CheckpointEligibility::named(EngineBoundary::OuterParagraphEnd),
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
        assert_ne!(
            owner(retention, CheckpointOwnerFamily::Mode),
            owner(later.retention(), CheckpointOwnerFamily::Mode),
            "each scalar mode checkpoint is its own bounded owner"
        );
        assert_eq!(
            owner(retention, CheckpointOwnerFamily::Core),
            owner(later.retention(), CheckpointOwnerFamily::Core),
            "fixed core marks share the one accepted lineage owner"
        );
    });
}

#[test]
fn retained_checkpoint_restores_command_tokens_and_scalar_mode_state() {
    crate::test_harness::with_nonstop_universe(|universe| {
        let command_root = universe
            .command_context()
            .expect("command context")
            .allocate_token_list(&[TokenWord::pack(Token::Char {
                ch: 'c',
                cat: Catcode::Other,
            })])
            .expect("command root");
        let mut command = CommandState::default();
        let mut diagnostic_effects = tex_state::diagnostic::DiagnosticEffects::new();
        command.push_everypar(
            &universe.command_context().expect("command context"),
            command_root,
        );
        let mut observations = ObservationRecorder::default();
        command.publish_named_token_list_pushes(
            &mut universe.command_context().expect("command context"),
            &mut diagnostic_effects,
            Some(&mut observations),
        );
        assert_eq!(
            observations.0.len(),
            1,
            "everypar publishes one retained push"
        );
        let mut modes = ModeNest::new();
        modes.current_list_mutation().set_prev_graf(7);
        let checkpoint = EngineCheckpoint::capture_checkpoint(
            CheckpointEligibility::job_start(),
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
        command.publish_named_token_list_pushes(
            &mut universe.command_context().expect("command context"),
            &mut diagnostic_effects,
            None,
        );
        modes.current_list_mutation().set_prev_graf(9);
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

        assert_eq!(modes.current_list().prev_graf(), 7);
    });
}

#[test]
fn retained_checkpoint_rejects_a_fresh_command_timeline_before_mutation() {
    crate::test_harness::with_nonstop_universe(|universe| {
        universe
            .assign_count(0, 10, AssignmentScope::Global)
            .expect("baseline count");
        let checkpoint = EngineCheckpoint::capture_checkpoint(
            CheckpointEligibility::job_start(),
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
            CheckpointEligibility::job_start(),
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
        let checkpoint = EngineCheckpoint::capture_checkpoint(
            CheckpointEligibility::named(EngineBoundary::OuterParagraphEnd),
            &mut command,
            &mut modes,
            universe,
            ExecutionBudgetCounters::default(),
        )
        .expect("checkpoint captures");

        modes.push(Mode::Horizontal).expect("horizontal mode");
        modes.push(Mode::Math).expect("math mode");
        modes
            .push(Mode::RestrictedHorizontal)
            .expect("later nested mode");
        assert_eq!(modes.maximum_saved_depth(), 2);
        checkpoint
            .restore_state(&mut command, &mut modes, universe)
            .expect("checkpoint restores");
        assert_eq!(modes.depth(), 1, "semantic mode summary rolled back");
        assert_eq!(
            modes.maximum_saved_depth(),
            2,
            "operational high-water survives rollback"
        );
    });
}

#[test]
fn rejected_mode_fork_returns_the_coarse_timeline_without_branch_nodes() {
    let store = retained_store();
    let mut accepted =
        RetainedEngineGeneration::new(&store, World::default()).expect("accepted generation");
    let checkpoint = accepted
        .with_admitted(CaptureModeCheckpoint {
            accepted_tail_len: 0,
        })
        .expect("checkpoint admission");

    let (mut rejected, runtime, _) = accepted
        .fork_checkpoint(&checkpoint)
        .expect("mode timeline forks");
    assert_eq!(
        rejected
            .with_admitted(InspectAndRejectModeFork {
                runtime,
                append_penalty: Some(22),
            })
            .expect("candidate admission"),
        [22],
        "the candidate sees only its detached branch from the eligible empty root"
    );
    drop(rejected);

    let (mut retried, runtime, _) = accepted
        .fork_checkpoint(&checkpoint)
        .expect("returned mode timeline forks again");
    assert_eq!(
        retried
            .with_admitted(InspectAndRejectModeFork {
                runtime,
                append_penalty: None,
            })
            .expect("retry admission"),
        Vec::<i32>::new(),
        "candidate-only nodes must not escape the rejected coarse owner"
    );
    drop(retried);
}

#[test]
fn early_rootless_fork_rejects_without_losing_the_large_accepted_head() {
    let store = retained_store();
    let mut accepted =
        RetainedEngineGeneration::new(&store, World::default()).expect("accepted generation");
    let checkpoint = accepted
        .with_admitted(CaptureModeCheckpoint {
            accepted_tail_len: 512,
        })
        .expect("early checkpoint admission");

    let (mut rejected, runtime, _) = accepted
        .fork_checkpoint(&checkpoint)
        .expect("early checkpoint forks");
    assert_eq!(
        rejected
            .with_admitted(InspectAndRejectModeFork {
                runtime,
                append_penalty: Some(900),
            })
            .expect("candidate admission"),
        [900],
        "the early checkpoint begins with a rootless current mode list"
    );
    drop(rejected);

    assert_eq!(
        accepted
            .with_admitted(PageContributionCount)
            .expect("restored source admission"),
        512,
        "rejection returns the untouched accepted page payload",
    );
    let (mut retried, runtime, _) = accepted
        .fork_checkpoint(&checkpoint)
        .expect("returned early checkpoint forks again");
    assert!(
        retried
            .with_admitted(InspectAndRejectModeFork {
                runtime,
                append_penalty: None,
            })
            .expect("retry admission")
            .is_empty()
    );
    drop(retried);
}
