#![cfg(feature = "testing")]

mod common;

use common::{Oracle, TestCell};
use proptest::prelude::*;
use proptest::test_runner::Config;
use std::env;
use tex_state::env::Env;
use tex_state::stores::{Snapshot, Stores};

#[derive(Clone, Debug)]
enum Op {
    Set {
        cell: TestCell,
        word: u64,
        global: bool,
    },
    EnterGroup,
    LeaveGroup,
    Checkpoint,
}

proptest! {
    #![proptest_config(Config {
        cases: prop_cases(),
        ..Config::default()
    })]

    #[test]
    fn replay_identity_matches_checkpoint_hashes(ops in balanced_ops()) {
        run_replay_identity(&ops);
    }
}

#[test]
fn group_exit_epoch_amendment_smoke() {
    let mut stores = Stores::new();
    let mut oracle = Oracle::new();
    let cell = TestCell::Count(11);

    stores.with_env_mut(Env::enter_group);
    oracle.enter_group();
    let outer_snapshot = stores.checkpoint();
    stores.with_env_mut(Env::enter_group);
    oracle.enter_group();
    stores.with_env_mut(|env| cell.set(env, 1, false));
    oracle.set(cell, 1, false);
    assert_eq!(stores.with_env_mut(Env::leave_group), Vec::<u64>::new());
    oracle.leave_group();

    // Shadow catches storage/barrier bypasses; this oracle assertion catches
    // semantic drift in group compaction and epoch handling (core_state §11).
    stores.with_env_mut(|env| cell.set(env, 2, false));
    oracle.set(cell, 2, false);
    oracle.assert_matches(stores.env(), &[cell]);
    verify_shadow(&stores);

    stores.rollback(outer_snapshot);
    assert_eq!(cell.get(stores.env()), 0);
    verify_shadow(&stores);
}

fn run_replay_identity(ops: &[Op]) {
    let mut stores = Stores::new();
    let mut oracle = Oracle::new();
    let cells = cell_universe();
    let mut checkpoints: Vec<(Snapshot, u64)> = Vec::new();
    let mut depth = 0_u8;

    let hash = stores.testing_state_hash();
    checkpoints.push((stores.checkpoint(), hash));
    for op in ops {
        match *op {
            Op::Set { cell, word, global } => {
                stores.with_env_mut(|env| cell.set(env, word, global));
                oracle.set(cell, word, global);
            }
            Op::EnterGroup => {
                stores.with_env_mut(Env::enter_group);
                oracle.enter_group();
                depth += 1;
            }
            Op::LeaveGroup => {
                assert_eq!(stores.with_env_mut(Env::leave_group), Vec::<u64>::new());
                oracle.leave_group();
                depth -= 1;
                oracle.assert_matches(stores.env(), &cells);
            }
            Op::Checkpoint if depth == 0 => {
                let hash = stores.testing_state_hash();
                checkpoints.push((stores.checkpoint(), hash));
            }
            Op::Checkpoint => {}
        }
        oracle.assert_matches(stores.env(), &cells);
        verify_shadow(&stores);
    }

    for (snapshot, hash) in checkpoints.into_iter().rev() {
        stores.rollback(snapshot);
        assert_eq!(
            stores.testing_state_hash(),
            hash,
            "rollback to {snapshot:?}"
        );
        verify_shadow(&stores);
    }
}

fn balanced_ops() -> impl Strategy<Value = Vec<Op>> {
    prop::collection::vec(op_seed(), 1..80).prop_map(|seeds| {
        let mut ops = Vec::with_capacity(seeds.len() + 8);
        let mut depth = 0_u8;
        for seed in seeds {
            match seed {
                OpSeed::Set { cell, word, global } => ops.push(Op::Set { cell, word, global }),
                OpSeed::EnterGroup => {
                    ops.push(Op::EnterGroup);
                    depth += 1;
                }
                OpSeed::LeaveGroup if depth > 0 => {
                    ops.push(Op::LeaveGroup);
                    depth -= 1;
                }
                OpSeed::LeaveGroup => {
                    ops.push(Op::EnterGroup);
                    depth += 1;
                }
                OpSeed::Checkpoint if depth == 0 => ops.push(Op::Checkpoint),
                OpSeed::Checkpoint => {}
            }
        }
        for _ in 0..depth {
            ops.push(Op::LeaveGroup);
        }
        ops
    })
}

#[derive(Clone, Debug)]
enum OpSeed {
    Set {
        cell: TestCell,
        word: u64,
        global: bool,
    },
    EnterGroup,
    LeaveGroup,
    Checkpoint,
}

fn op_seed() -> impl Strategy<Value = OpSeed> {
    prop_oneof![
        7 => (cell_strategy(), 0_u64..64, any::<bool>()).prop_map(|(cell, word, global)| {
            OpSeed::Set { cell, word, global }
        }),
        1 => Just(OpSeed::EnterGroup),
        1 => Just(OpSeed::LeaveGroup),
        2 => Just(OpSeed::Checkpoint),
    ]
}

fn cell_strategy() -> impl Strategy<Value = TestCell> {
    prop_oneof![
        (0_u32..8).prop_map(TestCell::Meaning),
        register_index().prop_map(TestCell::Count),
        register_index().prop_map(TestCell::Dimen),
        register_index().prop_map(TestCell::Skip),
        register_index().prop_map(TestCell::Toks),
        register_index().prop_map(TestCell::Box),
        (0_u16..16).prop_map(TestCell::IntParam),
        (0_u16..16).prop_map(TestCell::DimenParam),
        (0_u16..16).prop_map(TestCell::GlueParam),
        (0_u16..16).prop_map(TestCell::TokParam),
    ]
}

fn register_index() -> impl Strategy<Value = u16> {
    prop_oneof![
        3 => 0_u16..64,
        1 => 256_u16..320,
        1 => 32_704_u16..32_768,
    ]
}

fn cell_universe() -> Vec<TestCell> {
    let mut cells = Vec::new();
    for index in 0..8 {
        cells.push(TestCell::Meaning(index));
    }
    for index in (0..64).chain(256..320).chain(32_704..32_768) {
        cells.push(TestCell::Count(index));
        cells.push(TestCell::Dimen(index));
        cells.push(TestCell::Skip(index));
        cells.push(TestCell::Toks(index));
        cells.push(TestCell::Box(index));
    }
    for index in 0..16 {
        cells.push(TestCell::IntParam(index));
        cells.push(TestCell::DimenParam(index));
        cells.push(TestCell::GlueParam(index));
        cells.push(TestCell::TokParam(index));
    }
    cells
}

fn prop_cases() -> u32 {
    env::var("PROPTEST_CASES")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(256)
}

#[cfg(feature = "shadow")]
fn verify_shadow(stores: &Stores) {
    stores.verify_shadow();
}

#[cfg(not(feature = "shadow"))]
fn verify_shadow(_: &Stores) {}
