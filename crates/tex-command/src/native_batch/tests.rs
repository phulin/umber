use std::sync::Arc;

use tex_state::{GroupKind, TrackedRegionBarrier, TrackedRegionError, Universe, World};

use super::{
    NativeBatchBarrier, NativeBatchNodeSink, NativeBatchProgram, NativeBatchRequiredBarrier,
};
use crate::{CharacterCode, CommandProfile};

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

fn compile(source: &[u8], calls: usize) -> Result<NativeBatchProgram, NativeBatchBarrier> {
    let stores = Universe::new_with_plain_catcodes();
    NativeBatchProgram::compile(
        Arc::<[u8]>::from(source),
        CommandProfile::TEX82,
        stores.endlinechar(),
        |code: CharacterCode| {
            let byte = code.to_byte().expect("exact-byte profile");
            stores.catcode(char::from(byte))
        },
        calls,
    )
}

#[test]
fn canonical_lexer_feeds_grouped_assignment_macro_and_output_episode() {
    let source = br"\count0=0\count1=0\count2=0\def\e#1{\advance\count0by#1\global\advance\count1by#1\ifnum#1<5\global\advance\count2by1\else\global\advance\count2by2\fi A\kern#1sp}\shipout\hbox{\e{1}\e{2}\e{3}\e{4}\e{5}\e{6}\e{7}\e{8}}\end";
    let program = compile(source, 8).expect("supported program admits");
    let mut stores = Universe::new_with_plain_catcodes();
    let mut nodes = TestNodeSink::default();
    let outcome = program
        .execute(&mut stores, &mut nodes)
        .expect("admitted program executes");

    assert_eq!(outcome.counts, [0, 36, 12]);
    assert_eq!(
        [stores.count(0), stores.count(1), stores.count(2)],
        outcome.counts
    );
    assert_eq!(outcome.calls, 8);
    assert_eq!(nodes.0.len(), 16);
    assert_eq!(nodes.0[0], TestNode::Character(b'A'));
    assert_eq!(nodes.0[1], TestNode::Kern(1));
}

#[test]
fn native_and_scalar_count_group_paths_share_format_hash_and_restoration() {
    let source = br"\count0=10\begingroup\count0=20\count1=1\begingroup\count0=30\global\count1=7\count1=9\endgroup\count2=4\endgroup\end";
    let program = compile(source, 0).expect("group program admits");
    let mut native = Universe::new_with_plain_catcodes();
    let mut nodes = TestNodeSink::default();
    let outcome = program
        .execute(&mut native, &mut nodes)
        .expect("native group program executes");

    let mut scalar = Universe::new_with_plain_catcodes();
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
    assert_eq!(
        restored.dump_format().expect("restored format redumps"),
        image
    );
}

#[test]
fn canonical_snapshot_rolls_native_count_episode_back_exactly() {
    let program = compile(
        br"\count0=11\begingroup\count0=22\global\count1=33\count2=44\endgroup\end",
        0,
    )
    .expect("rollback program admits");
    let mut stores = Universe::new_with_plain_catcodes();
    stores.set_count(0, 5);
    stores.set_count(1, 6);
    stores.set_count(2, 7);
    let before_hash = stores.snapshot().state_hash();
    let before_format = stores.dump_format().expect("baseline format dumps");
    let checkpoint = stores.snapshot_with_exact_identity();

    program
        .execute(&mut stores, &mut TestNodeSink::default())
        .expect("native rollback program executes");
    assert_eq!(
        [stores.count(0), stores.count(1), stores.count(2)],
        [11, 33, 7]
    );
    stores.rollback(&checkpoint);

    assert_eq!(
        [stores.count(0), stores.count(1), stores.count(2)],
        [5, 6, 7]
    );
    assert_eq!(stores.snapshot().state_hash(), before_hash);
    assert_eq!(
        stores.dump_format().expect("rolled back format dumps"),
        before_format
    );
}

#[test]
fn active_incremental_observation_is_a_typed_episode_barrier() {
    let program = compile(br"\count0=9\end", 0).expect("tracked program admits");
    let mut stores = Universe::new_with_plain_catcodes();
    let tracked = stores
        .begin_tracked_region()
        .expect("tracked region begins");

    let error = program
        .execute(&mut stores, &mut TestNodeSink::default())
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
fn unsupported_control_sequence_stops_before_execution() {
    let error = compile(br"\count0=1\message{observable}\end", 0)
        .expect_err("observable command is outside the episode");
    assert_eq!(
        error,
        NativeBatchBarrier::Required(NativeBatchRequiredBarrier::Effect)
    );
}

#[test]
fn material_after_end_is_an_explicit_admission_barrier() {
    let error = compile(br"\end\relax", 0).expect_err("post-end material is refused");
    assert_eq!(error, NativeBatchBarrier::MaterialAfterEnd);
}
