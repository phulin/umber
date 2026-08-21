use std::sync::Arc;

use tex_command::{CommandObservation, CommandObserver, RegisteredSourceKind, SourceRegistration};

use super::{EpisodeCommitBoundary, SemanticEpisodeBarrier};
use crate::{AdvanceOutcome, AdvanceReadiness, MainControl, ResourceNeed, StepResult};

fn with_control<R>(
    source: &[u8],
    test: impl for<'id> FnOnce(
        &mut MainControl<tex_state::GenerationBrand<'id>>,
        &mut tex_state::Universe<tex_state::GenerationBrand<'id>>,
    ) -> R,
) -> R {
    crate::test_harness::with_plain_universe(|stores| {
        let mut control = MainControl::tex82_initex(stores);
        control
            .register_root_source(SourceRegistration::new(
                RegisteredSourceKind::Generated,
                Arc::<[u8]>::from(source),
            ))
            .expect("root source registers");
        test(&mut control, stores)
    })
}

#[derive(Default)]
struct Observer;

impl CommandObserver for Observer {
    fn committed(&mut self, _observation: CommandObservation) {}
}

#[test]
fn fixed_telemetry_counts_every_required_barrier() {
    let mut telemetry = super::EpisodeTelemetry::default();
    for barrier in [
        SemanticEpisodeBarrier::Resource,
        SemanticEpisodeBarrier::Effect,
        SemanticEpisodeBarrier::Observer,
        SemanticEpisodeBarrier::Diagnostic,
        SemanticEpisodeBarrier::Checkpoint,
        SemanticEpisodeBarrier::Format,
        SemanticEpisodeBarrier::Output,
        SemanticEpisodeBarrier::Cancellation,
        SemanticEpisodeBarrier::Fuel,
        SemanticEpisodeBarrier::StateIdentity,
    ] {
        telemetry.record_semantic_barrier(barrier);
        assert_eq!(telemetry.semantic_barriers(barrier), 1);
    }
}

#[test]
fn main_control_slice_is_bounded_and_groups_do_not_stop_the_episode() {
    let mut source = Vec::new();
    for _ in 0..300 {
        source.extend_from_slice(br"\relax");
    }
    source.extend_from_slice(br"\end");
    with_control(&source, |control, stores| {
        control
            .advance_episode(stores)
            .expect("closed slice commits");
        let telemetry = control.episode_telemetry();
        assert_eq!(telemetry.commits(), 1);
        assert_eq!(telemetry.operations(), 256);
        assert_eq!(telemetry.slice_limits(), 1);
        assert_eq!(
            telemetry.last_commit().expect("one commit").boundary(),
            EpisodeCommitBoundary::SliceLimit
        );
    });

    with_control(
        br"\begingroup\relax\endgroup\end",
        |grouped, grouped_stores| {
            grouped
                .advance_episode(grouped_stores)
                .expect("group-spanning episode commits");
            let telemetry = grouped.episode_telemetry();
            assert_eq!(telemetry.operations(), 4);
            crate::test_harness::with_admitted(grouped_stores, |context| {
                assert_eq!(context.execution_group_depth(), 0);
            });
        },
    );
}

#[test]
fn resource_fuel_observer_and_cancellation_return_without_state_drift() {
    with_control(br"\input child\end", |resource, stores| {
        let before = stores.journal_cursor().expect("live state cursor");
        let resource_step = resource.advance_episode(stores).expect("suspends");
        assert!(
            matches!(
                resource_step,
                StepResult::Suspended(ResourceNeed::Input { .. })
            ),
            "unexpected resource step: {resource_step:?}"
        );
        assert_eq!(stores.journal_cursor().expect("live state cursor"), before);
        assert_eq!(resource.episode_telemetry().rollbacks(), 1);
        assert_eq!(
            resource
                .episode_telemetry()
                .semantic_barriers(SemanticEpisodeBarrier::Resource),
            1
        );
    });

    with_control(br"\relax\end", |fuel, stores| {
        fuel.set_fuel_limit(1).expect("positive fuel limit");
        let before = stores.journal_cursor().expect("live state cursor");
        fuel.advance_episode(stores)
            .expect_err("fuel exhaustion is typed failure");
        assert_eq!(stores.journal_cursor().expect("live state cursor"), before);
        assert_eq!(
            fuel.episode_telemetry()
                .semantic_barriers(SemanticEpisodeBarrier::Fuel),
            1
        );
    });

    with_control(br"\relax\end", |observed, stores| {
        observed
            .advance_with_observer(stores, &mut Observer)
            .expect("observed episode commits");
        assert_eq!(
            observed
                .episode_telemetry()
                .semantic_barriers(SemanticEpisodeBarrier::Observer),
            1
        );
    });

    with_control(br"\relax\end", |cancelled, stores| {
        assert_eq!(
            cancelled
                .advance_when(stores, AdvanceReadiness::Cancelled)
                .expect("cancellation is not execution failure"),
            AdvanceOutcome::Cancelled
        );
        assert_eq!(
            cancelled
                .episode_telemetry()
                .semantic_barriers(SemanticEpisodeBarrier::Cancellation),
            1
        );
        assert_eq!(cancelled.episode_telemetry().attempts(), 0);
    });
}

#[test]
fn effect_diagnostic_format_and_state_identity_are_distinct_barriers() {
    with_control(br"\message{visible}\end", |effect, stores| {
        effect
            .advance_episode(stores)
            .expect("effect episode commits");
        assert_eq!(
            effect
                .episode_telemetry()
                .semantic_barriers(SemanticEpisodeBarrier::Effect),
            1
        );
    });

    with_control(br"\undefined\end", |diagnostic, stores| {
        stores.set_interaction_mode(tex_state::InteractionMode::Nonstop);
        diagnostic
            .advance_episode(stores)
            .expect("recoverable diagnostic commits");
        assert_eq!(
            diagnostic
                .episode_telemetry()
                .semantic_barriers(SemanticEpisodeBarrier::Diagnostic),
            1,
            "diagnostic telemetry: {:?}",
            diagnostic.episode_telemetry()
        );
    });

    with_control(br"\dump", |format, stores| {
        format
            .advance_episode(stores)
            .expect("INITEX format dump commits");
        assert!(format.dumped_format());
        assert_eq!(
            format
                .episode_telemetry()
                .semantic_barriers(SemanticEpisodeBarrier::Format),
            1
        );
    });

    with_control(br"\relax\end", |tracked, stores| {
        tracked
            .advance_with_tracked_region(stores)
            .expect("tracked operation commits");
        assert_eq!(
            tracked
                .episode_telemetry()
                .semantic_barriers(SemanticEpisodeBarrier::StateIdentity),
            1
        );
    });
}
