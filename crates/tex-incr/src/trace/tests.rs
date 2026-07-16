use std::cell::RefCell;
use std::collections::BTreeMap;

use super::*;

type Summary = TraceSummary<&'static str, i32, &'static str, &'static str, &'static str>;

fn leaf(
    operations: &[TraceOperation<&'static str, i32>],
    input: &'static str,
    effect: &'static str,
    output: &'static str,
) -> Summary {
    Summary::leaf(operations, &[input], &[effect], &[output]).expect("valid leaf")
}

#[test]
fn parent_excludes_only_reads_satisfied_by_earlier_child_writes() {
    let first = leaf(
        &[
            TraceOperation::Read { key: "a", value: 0 },
            TraceOperation::Write { key: "a", value: 1 },
        ],
        "input-1",
        "effect-1",
        "output-1",
    );
    let second = leaf(
        &[
            TraceOperation::Read { key: "a", value: 1 },
            TraceOperation::Read { key: "b", value: 7 },
            TraceOperation::Write { key: "a", value: 2 },
        ],
        "input-2",
        "effect-2",
        "output-2",
    );
    let parent = Summary::parent(&[first, second]).expect("composable parent");

    assert_eq!(parent.external_reads(), &[("a", 0), ("b", 7)]);
    assert_eq!(parent.redo(), &[("a", 1), ("a", 2)]);
    assert_eq!(parent.input(), &["input-1", "input-2"]);
    assert_eq!(parent.effects(), &["effect-1", "effect-2"]);
    assert_eq!(parent.outputs(), &["output-1", "output-2"]);
    assert_eq!(parent.leaf_count(), 2);
    assert!(parent.logical_bytes() >= std::mem::size_of::<Summary>());
}

#[test]
fn nested_parent_replay_matches_leaf_by_leaf_replay() {
    let leaves = [
        leaf(
            &[
                TraceOperation::Read { key: "a", value: 0 },
                TraceOperation::Write { key: "a", value: 1 },
            ],
            "input-1",
            "effect-1",
            "output-1",
        ),
        leaf(
            &[
                TraceOperation::Read { key: "a", value: 1 },
                TraceOperation::Write { key: "b", value: 3 },
            ],
            "input-2",
            "effect-2",
            "output-2",
        ),
        leaf(
            &[
                TraceOperation::Read { key: "b", value: 3 },
                TraceOperation::Write { key: "a", value: 2 },
            ],
            "input-3",
            "effect-3",
            "output-3",
        ),
    ];
    let nested = Summary::parent(&[
        Summary::parent(&leaves[..2]).expect("first parent"),
        Summary::parent(&leaves[2..]).expect("second parent"),
    ])
    .expect("nested parent");

    let replay = |summaries: &[Summary]| {
        let state = RefCell::new(BTreeMap::from([("a", 0)]));
        let writes = RefCell::new(Vec::new());
        let mut input = Vec::new();
        let mut effects = Vec::new();
        let mut outputs = Vec::new();
        for summary in summaries {
            summary
                .validate_and_replay(
                    |key| state.borrow().get(key).copied(),
                    |key, value| {
                        state.borrow_mut().insert(*key, *value);
                        writes.borrow_mut().push((*key, *value));
                    },
                    &mut input,
                    &mut effects,
                    &mut outputs,
                )
                .expect("valid replay");
        }
        (
            state.into_inner(),
            writes.into_inner(),
            input,
            effects,
            outputs,
        )
    };

    assert_eq!(replay(&leaves), replay(&[nested]));
}

#[test]
fn mismatched_internal_read_cannot_be_hidden_by_parent() {
    let writer = leaf(
        &[TraceOperation::Write { key: "a", value: 1 }],
        "input-1",
        "effect-1",
        "output-1",
    );
    let inconsistent = leaf(
        &[TraceOperation::Read { key: "a", value: 2 }],
        "input-2",
        "effect-2",
        "output-2",
    );

    assert_eq!(
        Summary::parent(&[writer, inconsistent]),
        Err(TraceCompositionError::InternalReadMismatch)
    );
}

#[test]
fn failed_parent_validation_is_an_atomic_miss() {
    let summary = leaf(
        &[
            TraceOperation::Read { key: "a", value: 0 },
            TraceOperation::Write { key: "a", value: 1 },
        ],
        "input",
        "effect",
        "output",
    );
    let state = RefCell::new(BTreeMap::from([("a", 9)]));
    let mut input = Vec::new();
    let mut effects = Vec::new();
    let mut outputs = Vec::new();

    assert_eq!(
        summary.validate_and_replay(
            |key| state.borrow().get(key).copied(),
            |key, value| {
                state.borrow_mut().insert(*key, *value);
            },
            &mut input,
            &mut effects,
            &mut outputs,
        ),
        Err(TraceValidationError { dependency: 0 })
    );
    assert_eq!(state.into_inner(), BTreeMap::from([("a", 9)]));
    assert!(input.is_empty());
    assert!(effects.is_empty());
    assert!(outputs.is_empty());
}
