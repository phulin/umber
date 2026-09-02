# `umber2-66p0.8.40.150`: row-owned input rollback epoch

## Adopted boundary

Every concrete `InputLevel` variant now directly owns its eight-byte packed
rollback epoch/state marker. `InputStack::rollback_markers` and its `Vec`
owner, capacity, admission, pop, retained-byte accounting, and wrap reset are
deleted. Admission initializes the incoming row header; source and resident
advancement mutate the marker reached through the already-selected row; cold
capture, replacement, retirement, and rollback likewise use that single
authoritative owner in constant time.

Replacement history swaps the semantic occupants while retaining the packed
marker on the physical resident row, preserving the former row-slot epoch
lifetime across candidate undo and redo. Epoch wrap resets the markers in the
existing row allocation. The marker is operational capture metadata and is
therefore deliberately invisible to semantic `Eq` and `Hash`, just as the
deleted side vector was.

The four-byte stored-span length moved from each replay, durable, and attempt
variant into the common `TokenCursor`'s four bytes of former tail padding.
This makes room for the row marker without enlarging the largest durable
payload. `InputLevel` retains an explicit compact discriminant and remains
exactly 88 bytes. No scan, map, cache, second representation, or allocation was
introduced.

## Focused mixed-input evidence

The exact baseline is commit `a59a4e230059acb0b6371a27931f84042c715ec8`;
the measured implementation is commit
`b9a8b4b24f49791348337da546ec66c0ea2d6820`. Both Rust 1.93.0
release/profiling executables ran `mixed_macro_resident_pipeline`: 2,000,000
macro-body transitions, 1,000,000 parameter deliveries, 1,000,004 replay
words, 2,000,004 raw frame steps, 1,000,000 expanded deliveries, 1,000,001
macro expansions, zero suspension moves, zero command copies, and zero warmed
allocations or requested bytes.

| Exact result                         |        Baseline |           Final |                  Delta |
| ------------------------------------ | --------------: | --------------: | ---------------------: |
| `InputLevel<()>`                     |        88 bytes |        88 bytes |              unchanged |
| `InputStack::push_row` code size     |     1,884 bytes |     1,735 bytes |          -149 (-7.91%) |
| Resident transition code size        |     4,993 bytes |     4,823 bytes |          -170 (-3.40%) |
| Complete executable text             | 2,120,437 bytes | 2,112,852 bytes |        -7,585 (-0.36%) |
| User instructions                    |   2,328,802,440 |   2,320,803,916 |    -7,998,524 (-0.34%) |
| User branches                        |     382,465,756 |     378,464,900 |    -4,000,856 (-1.05%) |
| User cycles                          |   1,715,257,353 |   1,055,720,085 | -659,537,268 (-38.45%) |
| Internal elapsed nanoseconds         |     615,344,165 |     406,152,505 | -209,191,660 (-34.00%) |
| Warmed allocations / requested bytes |           0 / 0 |           0 / 0 |              unchanged |
| Public `memcpy` calls / bytes        |   135 / 346,884 |   135 / 346,819 |                0 / -65 |
| Public `memmove` calls / bytes       |           2 / 0 |           2 / 0 |              unchanged |

The accepted deterministic results are the deleted owner, unchanged row size,
smaller named owners and executable text, 0.34% fewer instructions, 1.05%
fewer branches, and unchanged zero-allocation/copy topology. The single-run
cycle and elapsed reductions are recorded but are not used as a speed claim.
Both public-copy reports reconcile exactly with zero collision probes,
overflow, or probe-internal calls and attribute no copy to resident input
advancement.

Baseline and final binary SHA-256 values are respectively
`4427b79083314aeac107e02cf0d1fe95f255ebd897424efba6d1a792c0c0b5cf`
and
`fd3ddb969fc9363cfec5fdffff5faf9eecb3011c46edecc0eeed7e34d867358d`.
Their `perf stat` receipt hashes are
`5c3a7cb2a358b73974975360de94ce00a17b6ec24556b1e03937fdb0af5cb78a`
and
`50c4bcdc56500112f0efe3a8a9cb89e0450c91619ee05353f0910a85887ac4fe`;
their symbolized copy-report hashes are
`678e28e37ae44f477c954244e083e691f7a7379dc937c278dc50bb0097356e52`
and
`249b95b208bf72e5b2db30bcb9bb7b3874d185c37d0b9691ffb6296dd32e9495`.
The checked interposer hash is
`8afd6ca34d91c28fccbccff4273a2f47dfa61861d3bceb53cf2c14dc7d2be16e`.
Ignored issue-private evidence is under `target/umber2-66p0.8.40.150/`.

## Validation and disposition

- Six focused input-history tests, twelve rollback-filtered tests, and the two
  input architecture boundaries pass.
- `cargo test -q --tests -p tex-command`: 392 unit tests and 23 boundary tests
  pass.
- `cargo test -q --tests -p tex-command --features profiling`: 431 unit tests
  and 23 boundary tests pass.
- `scripts/check.sh`: all four gates pass; both Clippy resolutions are clean
  across 32 workspace members.

The measured target is removed without a distinct demonstrated remainder, so
no successor is filed.
