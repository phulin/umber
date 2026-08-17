use std::sync::Arc;

use tex_command::{CommandObservation, CommandObserver, RegisteredSourceKind, SourceRegistration};

use super::{
    CoverageFallbackSafety, EpisodeCommitBoundary, EpisodeCoverageFallback, EpisodeCoverageFamily,
    SemanticEpisodeBarrier,
};
use crate::{AdvanceOutcome, AdvanceReadiness, MainControl, ResourceNeed, StepResult};

fn control_for(source: &[u8]) -> (MainControl, tex_state::Universe) {
    let mut stores = tex_state::Universe::new_with_plain_catcodes();
    let mut control = MainControl::tex82_initex(&mut stores);
    control
        .register_root_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(source),
        ))
        .expect("root source registers");
    (control, stores)
}

#[derive(Default)]
struct Observer;

impl CommandObserver for Observer {
    fn committed(&mut self, _observation: CommandObservation) {}
}

#[test]
fn fixed_telemetry_counts_every_required_barrier_and_fallback_family() {
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
    for family in [
        EpisodeCoverageFamily::CharacterProfile,
        EpisodeCoverageFamily::SourceTokenization,
        EpisodeCoverageFamily::CommandVocabulary,
        EpisodeCoverageFamily::ScannerOrExpansion,
        EpisodeCoverageFamily::GroupLineage,
        EpisodeCoverageFamily::RollbackLineage,
    ] {
        let fallback = EpisodeCoverageFallback::mutation_free(family);
        assert_eq!(
            fallback.safety(),
            CoverageFallbackSafety::MutationFreeAdmission
        );
        telemetry.record_fallback(fallback);
        assert_eq!(telemetry.coverage_fallbacks(family), 1);
    }
}

#[test]
fn main_control_slice_and_group_boundary_are_typed_commits() {
    let mut source = Vec::new();
    for _ in 0..300 {
        source.extend_from_slice(br"\relax");
    }
    source.extend_from_slice(br"\end");
    let (mut control, mut stores) = control_for(&source);
    control
        .advance_episode(&mut stores)
        .expect("closed slice commits");
    let telemetry = control.episode_telemetry();
    assert_eq!(telemetry.commits(), 1);
    assert_eq!(telemetry.operations(), 256);
    assert_eq!(telemetry.slice_limits(), 1);
    assert_eq!(
        telemetry.last_commit().expect("one commit").boundary(),
        EpisodeCommitBoundary::SliceLimit
    );

    let (mut grouped, mut grouped_stores) = control_for(br"\begingroup\relax\endgroup\end");
    grouped
        .advance_episode(&mut grouped_stores)
        .expect("group entry commits");
    let telemetry = grouped.episode_telemetry();
    assert_eq!(
        telemetry.coverage_boundaries(EpisodeCoverageFamily::GroupLineage),
        1
    );
    assert_eq!(grouped_stores.group_depth(), 1);
}

#[test]
fn resource_fuel_observer_and_cancellation_return_without_state_drift() {
    let (mut resource, mut resource_stores) = control_for(br"\input child\end");
    let before = resource_stores.snapshot().state_hash();
    let resource_step = resource
        .advance_episode(&mut resource_stores)
        .expect("suspends");
    assert!(
        matches!(
            resource_step,
            StepResult::Suspended(ResourceNeed::Input { .. })
        ),
        "unexpected resource step: {resource_step:?}"
    );
    assert_eq!(resource_stores.snapshot().state_hash(), before);
    assert_eq!(resource.episode_telemetry().rollbacks(), 1);
    assert_eq!(
        resource
            .episode_telemetry()
            .semantic_barriers(SemanticEpisodeBarrier::Resource),
        1
    );

    let (mut fuel, mut fuel_stores) = control_for(br"\relax\end");
    fuel.set_fuel_limit(1).expect("positive fuel limit");
    let before = fuel_stores.snapshot().state_hash();
    fuel.advance_episode(&mut fuel_stores)
        .expect_err("fuel exhaustion is typed failure");
    assert_eq!(fuel_stores.snapshot().state_hash(), before);
    assert_eq!(
        fuel.episode_telemetry()
            .semantic_barriers(SemanticEpisodeBarrier::Fuel),
        1
    );

    let (mut observed, mut observed_stores) = control_for(br"\relax\end");
    observed
        .advance_with_observer(&mut observed_stores, &mut Observer)
        .expect("observed episode commits");
    assert_eq!(
        observed
            .episode_telemetry()
            .semantic_barriers(SemanticEpisodeBarrier::Observer),
        1
    );

    let (mut cancelled, mut cancelled_stores) = control_for(br"\relax\end");
    assert_eq!(
        cancelled
            .advance_when(&mut cancelled_stores, AdvanceReadiness::Cancelled)
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
}

#[test]
fn effect_diagnostic_format_and_state_identity_are_distinct_barriers() {
    let (mut effect, mut effect_stores) = control_for(br"\message{visible}\end");
    effect
        .advance_episode(&mut effect_stores)
        .expect("effect episode commits");
    assert_eq!(
        effect
            .episode_telemetry()
            .semantic_barriers(SemanticEpisodeBarrier::Effect),
        1
    );

    let (mut diagnostic, mut diagnostic_stores) = control_for(br"\undefined\end");
    diagnostic_stores.set_interaction_mode(tex_state::InteractionMode::Nonstop);
    diagnostic
        .advance_episode(&mut diagnostic_stores)
        .expect("recoverable diagnostic commits");
    assert_eq!(
        diagnostic
            .episode_telemetry()
            .semantic_barriers(SemanticEpisodeBarrier::Diagnostic),
        1,
        "diagnostic telemetry: {:?}",
        diagnostic.episode_telemetry()
    );

    let (mut format, mut format_stores) = control_for(br"\dump");
    format
        .advance_episode(&mut format_stores)
        .expect("INITEX format dump commits");
    assert!(format.dumped_format());
    assert_eq!(
        format
            .episode_telemetry()
            .semantic_barriers(SemanticEpisodeBarrier::Format),
        1
    );

    let (mut tracked, mut tracked_stores) = control_for(br"\relax\end");
    tracked
        .advance_with_tracked_region(&mut tracked_stores)
        .expect("tracked operation commits");
    assert_eq!(
        tracked
            .episode_telemetry()
            .semantic_barriers(SemanticEpisodeBarrier::StateIdentity),
        1
    );
}
