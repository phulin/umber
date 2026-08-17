use std::sync::Arc;

use tex_state::meaning::{Meaning, UnexpandablePrimitive};
use tex_state::{GroupKind, TrackedRegionBarrier, TrackedRegionError, Universe, World};

use super::{
    NativeBatchBarrier, NativeBatchNodeSink, NativeBatchProgram, NativeBatchRequiredBarrier,
};
use crate::{
    CommandHostCapabilities, CommandHostContext, CommandProcessor, CommandProfile, CommandState,
    RegisteredSourceKind, SourceRegistration,
};

#[derive(Debug, Eq, PartialEq)]
enum TestNode {
    Character(u8),
    Kern(i32),
}

#[derive(Default)]
struct TestNodeSink(Vec<TestNode>);

impl NativeBatchNodeSink for TestNodeSink {
    fn reserve(&mut self, additional: usize) {
        self.0.reserve(additional);
    }

    fn character(&mut self, ch: u8) {
        self.0.push(TestNode::Character(ch));
    }

    fn kern(&mut self, amount: i32) {
        self.0.push(TestNode::Kern(amount));
    }
}

fn install(stores: &mut Universe, name: &str, primitive: UnexpandablePrimitive) {
    let symbol = stores.intern_relaxed_control_sequence(name);
    stores.set_meaning_global(symbol, Meaning::UnexpandablePrimitive(primitive));
}

fn rig(source: &[u8]) -> (CommandState, Universe, CommandHostCapabilities) {
    let mut stores = Universe::new_with_plain_catcodes();
    crate::install_tex82_expandable_primitives(&mut stores);
    for (name, primitive) in [
        ("count", UnexpandablePrimitive::Count),
        ("advance", UnexpandablePrimitive::Advance),
        ("global", UnexpandablePrimitive::Global),
        ("shipout", UnexpandablePrimitive::Shipout),
        ("hbox", UnexpandablePrimitive::HBox),
        ("kern", UnexpandablePrimitive::Kern),
        ("end", UnexpandablePrimitive::End),
        ("begingroup", UnexpandablePrimitive::BeginGroup),
        ("endgroup", UnexpandablePrimitive::EndGroup),
        ("message", UnexpandablePrimitive::Message),
    ] {
        install(&mut stores, name, primitive);
    }
    let mut command = CommandState::new(CommandProfile::TEX82);
    let source = command
        .register_source(SourceRegistration::new(
            RegisteredSourceKind::Generated,
            Arc::<[u8]>::from(source),
        ))
        .expect("source registers");
    command
        .open_registered_source(source)
        .expect("source opens");
    (command, stores, CommandHostCapabilities::default())
}

fn execute(
    command: &mut CommandState,
    stores: &mut Universe,
    capabilities: &mut CommandHostCapabilities,
    calls: usize,
    nodes: &mut TestNodeSink,
) -> Result<super::NativeBatchOutcome, NativeBatchBarrier> {
    let mut processor = CommandProcessor::new(
        command,
        stores.command_context(),
        CommandHostContext::new(capabilities),
    );
    NativeBatchProgram::new(calls).execute(&mut processor, nodes)
}

#[test]
fn canonical_input_feeds_grouped_assignment_and_output_episode() {
    let source = br"\count0=1\count1=2\begingroup\count0=9\global\count1=7\endgroup\shipout\hbox{A\kern3sp}\end";
    let (mut command, mut stores, mut capabilities) = rig(source);
    let mut nodes = TestNodeSink::default();
    let outcome = execute(&mut command, &mut stores, &mut capabilities, 1, &mut nodes)
        .expect("canonical-input episode executes");

    assert_eq!(outcome.counts, [1, 7, 0]);
    assert_eq!(
        [stores.count(0), stores.count(1), stores.count(2)],
        outcome.counts
    );
    assert_eq!(outcome.calls, 1);
    assert_eq!(nodes.0, [TestNode::Character(b'A'), TestNode::Kern(3)]);
}

#[test]
fn canonical_and_scalar_count_group_paths_share_format_hash_and_restoration() {
    let source = br"\count0=10\begingroup\count0=20\count1=1\begingroup\count0=30\global\count1=7\count1=9\endgroup\count2=4\endgroup\end";
    let (mut command, mut native, mut capabilities) = rig(source);
    let outcome = execute(
        &mut command,
        &mut native,
        &mut capabilities,
        0,
        &mut TestNodeSink::default(),
    )
    .expect("native group program executes");

    let (_, mut scalar, _) = rig(b"");
    scalar.set_count(0, 10);
    scalar.enter_group_with_kind(GroupKind::SemiSimple);
    scalar.set_count(0, 20);
    scalar.set_count(1, 1);
    scalar.enter_group_with_kind(GroupKind::SemiSimple);
    scalar.set_count(0, 30);
    scalar.set_count_global(1, 7);
    scalar.set_count(1, 9);
    scalar
        .leave_group_with_kind(GroupKind::SemiSimple)
        .expect("inner scalar group closes");
    scalar.set_count(2, 4);
    scalar
        .leave_group_with_kind(GroupKind::SemiSimple)
        .expect("outer scalar group closes");

    assert_eq!(outcome.counts, [10, 7, 0]);
    assert_eq!(native.group_depth(), 0);
    assert_eq!(
        native.dump_format().expect("native format dumps"),
        scalar.dump_format().expect("scalar format dumps")
    );
    assert_eq!(
        native.snapshot().state_hash(),
        scalar.snapshot().state_hash()
    );

    let image = native.dump_format().expect("native format redumps");
    let restored = Universe::from_format(World::memory(), &image).expect("format restores");
    assert_eq!(
        [restored.count(0), restored.count(1), restored.count(2)],
        outcome.counts
    );
}

#[test]
fn active_incremental_observation_is_a_typed_episode_barrier() {
    let (mut command, mut stores, mut capabilities) = rig(br"\count0=9\end");
    let tracked = stores
        .begin_tracked_region()
        .expect("tracked region begins");

    let error = execute(
        &mut command,
        &mut stores,
        &mut capabilities,
        0,
        &mut TestNodeSink::default(),
    )
    .expect_err("native episode refuses active observation");

    assert_eq!(
        error,
        NativeBatchBarrier::State(tex_state::CountGroupEpisodeBarrier::ActiveTrackedRegion)
    );
    assert_eq!(stores.count(0), 0);
    assert!(matches!(
        stores.finish_tracked_region(tracked),
        Err(TrackedRegionError::UnsupportedRegion(
            TrackedRegionBarrier::UnsupportedExecutionState
        ))
    ));
}

#[test]
fn required_effect_is_classified_after_canonical_delivery() {
    let (mut command, mut stores, mut capabilities) = rig(br"\message{observable}\end");
    let error = execute(
        &mut command,
        &mut stores,
        &mut capabilities,
        0,
        &mut TestNodeSink::default(),
    )
    .expect_err("observable command is outside the episode");
    assert_eq!(
        error,
        NativeBatchBarrier::Required(NativeBatchRequiredBarrier::Effect)
    );
}

#[test]
fn root_completion_is_not_source_tokenization_fallback() {
    let (mut command, mut stores, mut capabilities) = rig(b"");
    let error = execute(
        &mut command,
        &mut stores,
        &mut capabilities,
        0,
        &mut TestNodeSink::default(),
    )
    .expect_err("root EOF returns to main control");
    assert_eq!(error, NativeBatchBarrier::RootCompletion);
}
