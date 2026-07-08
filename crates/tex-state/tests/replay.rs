#![cfg(feature = "testing")]

mod common;

use common::{Oracle, TestCell};
use proptest::prelude::*;
use proptest::test_runner::Config;
use std::collections::HashMap;
use std::env;
use std::time::Instant;
use tex_state::glue::{GlueSpec, Order};
use tex_state::ids::{FontId, GlueId, NodeListId};
use tex_state::node::{BoxNode, BoxNodeFields, GlueKind, Node, Sign};
use tex_state::scaled::Scaled;
use tex_state::stores::{Snapshot, Stores};
use tex_state::token::{Catcode, Token};

const TREE_FROM_STORE_MAX_DEPTH: usize = 4096;

#[derive(Clone, Debug)]
enum Op {
    Set {
        cell: TestCell,
        word: u64,
        global: bool,
    },
    InternTokens(Vec<Token>),
    InternGlue(GlueSpec),
    BuildNodes(NodeSeed),
    SetBoxReg {
        index: u16,
        list: usize,
        global: bool,
    },
    TakeBoxReg(u16),
    EnterGroup,
    LeaveGroup,
    Checkpoint,
}

#[derive(Clone, Debug)]
struct NodeSeed {
    ch: char,
    amount: i32,
    glue_slot: usize,
    nest_slot: Option<usize>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum TreeNode {
    Char { font: u32, ch: char },
    Kern(i32),
    Glue(GlueSpec, GlueKind),
    HList(TreeList),
    MathOn,
}

type TreeList = Vec<TreeNode>;

#[derive(Clone, Debug)]
struct BuiltList {
    id: NodeListId,
    tree: TreeList,
}

#[derive(Clone, Debug)]
struct Checkpoint {
    snapshot: Snapshot,
    hash: u64,
    survivor_slots: usize,
    boxes: BoxOracle,
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

    stores.enter_group();
    oracle.enter_group();
    let outer_snapshot = stores.checkpoint();
    stores.enter_group();
    oracle.enter_group();
    cell.set(&mut stores, 1, false);
    oracle.set(cell, 1, false);
    assert_eq!(stores.leave_group(), Vec::<Token>::new());
    oracle.leave_group();

    // Shadow catches storage/barrier bypasses; this oracle assertion catches
    // semantic drift in group compaction and epoch handling (core_state §11).
    cell.set(&mut stores, 2, false);
    oracle.set(cell, 2, false);
    oracle.assert_matches(stores.env(), &[cell]);
    verify_shadow(&stores);

    stores.rollback(outer_snapshot);
    assert_eq!(cell.get(stores.env()), 0);
    verify_shadow(&stores);
}

#[test]
fn rollback_keeps_box_register_ids_resolvable() {
    let mut stores = Stores::new();
    let baseline = stores.freeze_node_list(&[Node::MathOn]);
    stores.set_box_reg(0, baseline);
    let snapshot = stores.checkpoint();
    let temporary = stores.freeze_node_list(&[Node::MathOff]);
    stores.set_box_reg(0, temporary);
    stores.set_box_reg(257, temporary);

    stores.rollback(snapshot);

    // core_state §9's "restore as one tuple" is observable here: if the
    // journal were restored without the matching watermarks/refcounts, a box
    // register could hold a dangling survivor id and this resolve would panic.
    for index in (0..256).chain([257, 513, 32_767]) {
        if let Some(id) = stores.box_reg(index) {
            let _ = stores.nodes(id);
        }
    }
}

#[allow(clippy::disallowed_methods)]
fn run_replay_identity(ops: &[Op]) {
    let started = Instant::now();
    let mut stores = Stores::new();
    let mut oracle = Oracle::new();
    let mut box_oracle = BoxOracle::new();
    let mut glue_ids = vec![GlueId::ZERO];
    let mut built_lists = Vec::new();
    let cells = cell_universe();
    TestCell::prepare_stores(&mut stores, &cells);
    let mut checkpoints = Vec::new();
    let mut depth = 0_u8;

    let hash = stores.testing_state_hash();
    checkpoints.push(Checkpoint {
        snapshot: stores.checkpoint(),
        hash,
        survivor_slots: stores.testing_live_survivor_slot_count(),
        boxes: box_oracle.clone(),
    });

    for op in ops {
        match op {
            Op::Set { cell, word, global } => {
                cell.set(&mut stores, *word, *global);
                oracle.set(*cell, *word, *global);
            }
            Op::InternTokens(tokens) => {
                stores.intern_token_list(tokens);
            }
            Op::InternGlue(spec) => {
                glue_ids.push(stores.intern_glue(*spec));
            }
            Op::BuildNodes(seed) => {
                let list = build_nodes(&mut stores, &glue_ids, &built_lists, seed);
                built_lists.push(list);
            }
            Op::SetBoxReg {
                index,
                list,
                global,
            } => {
                if let Some(list) = choose_list(&built_lists, *list) {
                    if *global {
                        stores.set_box_reg_global(*index, list.id);
                    } else {
                        stores.set_box_reg(*index, list.id);
                    }
                    box_oracle.set(*index, Some(list.tree.clone()), *global);
                }
            }
            Op::TakeBoxReg(index) => {
                stores.take_box_reg(*index);
                box_oracle.set(*index, None, false);
            }
            Op::EnterGroup => {
                stores.enter_group();
                oracle.enter_group();
                box_oracle.enter_group();
                depth += 1;
            }
            Op::LeaveGroup => {
                assert_eq!(stores.leave_group(), Vec::<Token>::new());
                oracle.leave_group();
                box_oracle.leave_group();
                depth -= 1;
                oracle.assert_matches(stores.env(), &cells);
            }
            Op::Checkpoint if depth == 0 => {
                let hash = stores.testing_state_hash();
                checkpoints.push(Checkpoint {
                    snapshot: stores.checkpoint(),
                    hash,
                    survivor_slots: stores.testing_live_survivor_slot_count(),
                    boxes: box_oracle.clone(),
                });
            }
            Op::Checkpoint => {}
        }
        oracle.assert_matches(stores.env(), &cells);
        box_oracle.assert_matches(&stores);
        verify_shadow(&stores);
    }

    for checkpoint in checkpoints.into_iter().rev() {
        stores.rollback(checkpoint.snapshot.clone());
        assert_eq!(
            stores.testing_state_hash(),
            checkpoint.hash,
            "rollback to {:?}",
            checkpoint.snapshot
        );
        assert_eq!(
            stores.testing_live_survivor_slot_count(),
            checkpoint.survivor_slots,
            "survivor slot leak across rollback to {:?}",
            checkpoint.snapshot
        );
        checkpoint.boxes.assert_matches(&stores);
        verify_shadow(&stores);
    }

    eprintln!(
        "replay_identity cases={} ops={} elapsed={:?}",
        prop_cases(),
        ops.len(),
        started.elapsed()
    );
}

fn build_nodes(
    stores: &mut Stores,
    glue_ids: &[GlueId],
    built: &[BuiltList],
    seed: &NodeSeed,
) -> BuiltList {
    let glue_id = glue_ids[seed.glue_slot % glue_ids.len()];
    let glue = stores.glue(glue_id);
    let mut nodes = vec![
        Node::Char {
            font: FontId::testing_new(0),
            ch: seed.ch,
        },
        Node::Kern {
            amount: Scaled::from_raw(seed.amount),
            kind: tex_state::node::KernKind::Explicit,
        },
        Node::Glue {
            spec: glue_id,
            kind: GlueKind::Normal,
        },
    ];
    let mut tree = vec![
        TreeNode::Char {
            font: 0,
            ch: seed.ch,
        },
        TreeNode::Kern(seed.amount),
        TreeNode::Glue(glue, GlueKind::Normal),
    ];

    if let Some(slot) = seed.nest_slot.and_then(|slot| choose_list(built, slot)) {
        nodes.push(Node::HList(BoxNode::new(BoxNodeFields {
            width: Scaled::from_raw(1),
            height: Scaled::from_raw(2),
            depth: Scaled::from_raw(3),
            shift: Scaled::from_raw(4),
            glue_set: 0.0,
            glue_sign: Sign::Normal,
            glue_order: Order::Normal,
            children: slot.id,
        })));
        tree.push(TreeNode::HList(slot.tree.clone()));
    }

    BuiltList {
        id: stores.freeze_node_list(&nodes),
        tree,
    }
}

fn choose_list(lists: &[BuiltList], slot: usize) -> Option<&BuiltList> {
    (!lists.is_empty()).then(|| &lists[slot % lists.len()])
}

fn balanced_ops() -> impl Strategy<Value = Vec<Op>> {
    prop::collection::vec(op_seed(), 1..80).prop_map(|seeds| {
        let mut ops = Vec::with_capacity(seeds.len() + 8);
        let mut depth = 0_u8;
        for seed in seeds {
            match seed {
                OpSeed::Op(op) => ops.push(op),
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
    Op(Op),
    EnterGroup,
    LeaveGroup,
    Checkpoint,
}

fn op_seed() -> impl Strategy<Value = OpSeed> {
    prop_oneof![
        7 => (cell_strategy(), 0_u64..64, any::<bool>()).prop_map(|(cell, word, global)| {
            OpSeed::Op(Op::Set { cell, word, global })
        }),
        2 => prop::collection::vec(token_strategy(), 0..5)
            .prop_map(|tokens| OpSeed::Op(Op::InternTokens(tokens))),
        2 => glue_spec_strategy().prop_map(|spec| OpSeed::Op(Op::InternGlue(spec))),
        3 => node_seed_strategy().prop_map(|seed| OpSeed::Op(Op::BuildNodes(seed))),
        3 => (register_index(), 0_usize..32, any::<bool>()).prop_map(|(index, list, global)| {
            OpSeed::Op(Op::SetBoxReg { index, list, global })
        }),
        1 => register_index().prop_map(|index| OpSeed::Op(Op::TakeBoxReg(index))),
        1 => Just(OpSeed::EnterGroup),
        1 => Just(OpSeed::LeaveGroup),
        2 => Just(OpSeed::Checkpoint),
    ]
}

fn token_strategy() -> impl Strategy<Value = Token> {
    prop_oneof![
        (b'a'..=b'z').prop_map(|ch| Token::Char {
            ch: char::from(ch),
            cat: Catcode::Letter,
        }),
        (1_u8..=9).prop_map(Token::param),
    ]
}

fn glue_spec_strategy() -> impl Strategy<Value = GlueSpec> {
    (-100_i32..100).prop_map(|raw| GlueSpec {
        width: Scaled::from_raw(raw),
        stretch: Scaled::from_raw(raw.saturating_mul(2)),
        stretch_order: Order::Fil,
        shrink: Scaled::from_raw(raw.saturating_mul(3)),
        shrink_order: Order::Fill,
    })
}

fn node_seed_strategy() -> impl Strategy<Value = NodeSeed> {
    (
        b'a'..=b'z',
        -100_i32..100,
        0_usize..32,
        prop::option::of(0_usize..32),
    )
        .prop_map(|(ch, amount, glue_slot, nest_slot)| NodeSeed {
            ch: char::from(ch),
            amount,
            glue_slot,
            nest_slot,
        })
}

fn cell_strategy() -> impl Strategy<Value = TestCell> {
    prop_oneof![
        (0_u32..8).prop_map(TestCell::Meaning),
        register_index().prop_map(TestCell::Count),
        register_index().prop_map(TestCell::Dimen),
        register_index().prop_map(TestCell::Skip),
        register_index().prop_map(TestCell::Muskip),
        register_index().prop_map(TestCell::Toks),
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
        cells.push(TestCell::Muskip(index));
        cells.push(TestCell::Toks(index));
    }
    for index in 0..16 {
        cells.push(TestCell::IntParam(index));
        cells.push(TestCell::DimenParam(index));
        cells.push(TestCell::GlueParam(index));
        cells.push(TestCell::TokParam(index));
    }
    cells
}

#[derive(Clone, Debug)]
struct BoxOracle {
    scopes: Vec<HashMap<u16, Option<TreeList>>>,
}

impl BoxOracle {
    fn new() -> Self {
        Self {
            scopes: vec![HashMap::new()],
        }
    }

    fn enter_group(&mut self) {
        self.scopes.push(HashMap::new());
    }

    fn leave_group(&mut self) {
        assert!(self.scopes.len() > 1, "box oracle group underflow");
        self.scopes.pop();
    }

    fn set(&mut self, index: u16, value: Option<TreeList>, global: bool) {
        if global {
            for scope in &mut self.scopes {
                scope.insert(index, value.clone());
            }
        } else {
            self.scopes
                .last_mut()
                .expect("box oracle has a root scope")
                .insert(index, value);
        }
    }

    fn get(&self, index: u16) -> Option<&TreeList> {
        self.scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(&index))
            .and_then(Option::as_ref)
    }

    fn assert_matches(&self, stores: &Stores) {
        for index in (0..256).chain(256..320).chain(32_704..32_768) {
            let real = stores.box_reg(index).map(|id| tree_from_store(stores, id));
            assert_eq!(
                real.as_ref(),
                self.get(index),
                "box oracle mismatch at {index}"
            );
        }
    }
}

fn tree_from_store(stores: &Stores, id: NodeListId) -> TreeList {
    tree_from_store_bounded(stores, id, 0)
}

fn tree_from_store_bounded(stores: &Stores, id: NodeListId, depth: usize) -> TreeList {
    assert!(
        depth <= TREE_FROM_STORE_MAX_DEPTH,
        "replay oracle exceeded maximum node-list nesting depth"
    );
    stores
        .nodes(id)
        .iter()
        .map(|node| match node {
            Node::Char { font, ch } => TreeNode::Char {
                font: font.raw(),
                ch: *ch,
            },
            Node::Kern { amount, .. } => TreeNode::Kern(amount.raw()),
            Node::Glue { spec, kind } => TreeNode::Glue(stores.glue(*spec), *kind),
            Node::HList(box_node) => TreeNode::HList(tree_from_store_bounded(
                stores,
                box_node.children,
                depth + 1,
            )),
            Node::MathOn => TreeNode::MathOn,
            other => panic!("unexpected replay node: {other:?}"),
        })
        .collect()
}

fn prop_cases() -> u32 {
    env::var("PROPTEST_CASES")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(100)
}

#[cfg(feature = "shadow")]
fn verify_shadow(stores: &Stores) {
    stores.verify_shadow();
}

#[cfg(not(feature = "shadow"))]
fn verify_shadow(_: &Stores) {}
