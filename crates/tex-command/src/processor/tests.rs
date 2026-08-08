use super::CommandProcessor;
use crate::{CommandHostCapabilities, CommandHostContext, CommandState};
use tex_state::{
    DependencyEngineField, DependencyKey, TrackedRegionBarrier, TrackedRegionError, Universe,
};

#[test]
fn processor_observes_command_roots_once() {
    let mut command = CommandState::default();
    let mut universe = Universe::new();
    let mut host = CommandHostCapabilities::default();
    let mark = universe.begin_tracked_region().expect("start region");
    drop(CommandProcessor::new(
        &mut command,
        universe.command_context(),
        CommandHostContext::new(&mut host),
    ));
    let record = universe.finish_tracked_region(mark).expect("finish region");
    let keys = record
        .observations()
        .iter()
        .map(|observation| observation.key)
        .collect::<Vec<_>>();
    for expected in [
        DependencyKey::InputLine,
        DependencyKey::InputStack,
        DependencyKey::Engine(DependencyEngineField::ConditionLevel),
        DependencyKey::Engine(DependencyEngineField::ConditionType),
        DependencyKey::Engine(DependencyEngineField::ConditionBranch),
        DependencyKey::Engine(DependencyEngineField::ConditionStack),
    ] {
        assert_eq!(keys.iter().filter(|&&key| key == expected).count(), 1);
    }
}

#[test]
fn unsupported_command_continuation_fails_closed() {
    let mut command = CommandState {
        name_in_progress: true,
        ..CommandState::default()
    };
    let mut universe = Universe::new();
    let mut host = CommandHostCapabilities::default();
    let mark = universe.begin_tracked_region().expect("start region");
    drop(CommandProcessor::new(
        &mut command,
        universe.command_context(),
        CommandHostContext::new(&mut host),
    ));
    assert_eq!(
        universe.finish_tracked_region(mark),
        Err(TrackedRegionError::UnsupportedRegion(
            TrackedRegionBarrier::UnsupportedCommandState
        ))
    );
    assert!(!universe.dependency_region_is_active());
}
