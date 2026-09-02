# `umber2-66p0.8.40.148`: input rollback epoch lane

## Adopted boundary

`InputStack` now keeps one row-aligned eight-byte `RowRollbackMarker` lane.
The marker packs a 62-bit capture epoch with one of three reachable current-
epoch classes: admitted without an inverse, inline cursor/source-lexer inverse,
or complete cold token/source-owner inverse. An epoch mismatch means the row
predates the active capture. These four cases preserve the former replacement,
ordinary first-touch, and rare cold-capture decisions without the parallel
`touched`, `partially_captured`, and `cold_state_captured` vectors.

Push, resident advancement, and pop index only the selected marker and update
it in constant time. Warm row reuse remains allocation-free and scan-free;
rollback interval wrap retains the pre-existing exceptional reset behavior.
Nested checkpoint candidate undo/redo still swaps the singular ordered
`InputUndo` journal, so suspension, recovery insertion, source retirement, and
source-owner restoration acquire no second representation, map, cache, or
generation census. Marker storage falls from 24 to 8 bytes per row-capacity
slot, and `InputStack` drops two `Vec` owners.

## Focused mixed-input evidence

The exact base and final release/profiling executables ran the existing
`mixed_macro_resident_pipeline`. Both reported 2,000,000 macro-body
transitions, 1,000,000 parameter deliveries, 1,000,004 replay words,
2,000,004 raw frame steps, 1,000,000 expanded deliveries, 1,000,001 macro
expansions, zero suspension moves, zero command copies, and zero warmed
allocations or requested bytes.

| Exact result                         |      Baseline |         Final |                 Delta |
| ------------------------------------ | ------------: | ------------: | --------------------: |
| `InputStack::push_row` code size     |   2,158 bytes |   1,884 bytes |        -274 (-12.70%) |
| Resident transition code size        |   5,178 bytes |   4,993 bytes |         -185 (-3.57%) |
| User instructions                    | 2,370,769,783 | 2,324,766,663 |  -46,003,120 (-1.94%) |
| User branches                        |   389,199,423 |   381,198,719 |   -8,000,704 (-2.06%) |
| User cycles                          | 1,657,637,921 | 1,663,221,085 |   +5,583,164 (+0.34%) |
| Internal elapsed nanoseconds         |   708,124,653 |   611,124,078 | -97,000,575 (-13.70%) |
| Warmed allocations / requested bytes |         0 / 0 |         0 / 0 |             unchanged |
| Public `memcpy` calls / bytes        | 136 / 347,037 | 136 / 346,986 |               0 / -51 |
| Public `memmove` calls / bytes       |         2 / 0 |         2 / 0 |             unchanged |

The material result is the 1.94% instruction reduction on an exact unchanged
mixed-input work vector, reinforced by the 12.70% smaller push owner and 3.57%
smaller resident owner. The single-run cycle counter is effectively flat and
is not used as a speed claim. Both public-copy reports reconcile with zero
collisions, overflow, or probe-internal calls and attribute no new copy owner
to input rollback.

The base/final binary SHA-256 values are
`a0612fe20d429d137c41201e352722aea411c498fdc5ddb8d02844ca7ea98de0`
and
`3e5018442331678f9eea5d8efd2489c2983606869cc4328f53cd14a2025b2898`.
Their `perf stat` receipt hashes are
`befd5640a9bca9ad76d18519a2cf7ac49c5a9bbd57689199b41b74570465fcdd`
and
`41ad6aaf427b8625793dfc25b5a6b0578250f1f940f6a6f053c88f88f36a2d0b`;
their symbolized copy-report hashes are
`6c0048edf27b008c0770a821b7a28cdebefeb87838278a399b737a1db289ee55`
and
`9c5b3b3cc997a6d6c78a84ca79c055345fcf9ca3018f5c0224bad5e45f794283`.
The checked interposer hash is
`8afd6ca34d91c28fccbccff4273a2f47dfa61861d3bceb53cf2c14dc7d2be16e`.
Ignored evidence is under `target/umber2-66p0.8.40.148/`.

## Validation

Focused history, source-replacement, retirement, checkpoint, recovery,
suspension, nested-attempt, and architecture tests cover the exact lifecycle.

- Six input-history tests and the singular-history architecture boundary pass.
- The profiling allocation test plus six focused source-owner, checkpoint,
  suspension, recovery, nested-attempt, and retirement tests pass.
- `scripts/check.sh`: all four gates pass; both Clippy resolutions are clean
  across 32 workspace members.
