# `umber2-66p0.8.40.133`: checkpoint the string-pool frontier, not its owner

## Joint copy audit

The authenticated `.132` authority reported 7,287,659 public `memcpy` calls
for 1,080,804,810 bytes and 238,872 public `memmove` calls for 40,776,774
bytes, with zero overflow and zero probe-internal calls. The active node-view
and 32-byte record cutover owns the leading node rows. Its direct line-break
view callback alone owns 115,040 `memmove` calls and 19,326,720 bytes, while
`PageMaterialArena::push_active_list` owns another 113,493 `memmove` calls and
19,066,824 bytes. Those rows and deferred output parity were not changed.

The largest independent checkpoint copy consisted of two symbolized
`RecycledStringPool::clone` rows beneath
`Universe::runtime_checkpoint_with_page_roots_and_identity`: 24 calls and
23,275,841 bytes for the primary dense lane plus 24 calls and 6,291,456 bytes
for the second lane. Together, 48 calls copied 29,567,297 bytes solely so an
aggregate checkpoint could retain append-only string membership.

The audit also found a larger cross-component external-resource/PDF-image
chain totaling at least 112,132,392 `memcpy` bytes across resolver, World,
host, and PDF-state owners. That distinct lifetime is recorded once as P1
`umber2-66p0.8.40.134`; it is not mixed into this checkpoint change.

## Ownership change

`RuntimeCheckpoint` now stores the scalar `EngineUsageCheckpoint`, including
the recycled pool's byte and entry frontiers, rather than cloning
`EngineUsageRuntime` and its three vectors. Ordinary checkpoint capture and
checkpoint clone therefore retain no string-pool payload owner.

Generation-candidate fork moves the one live `EngineUsageRuntime` into the
candidate. The pool detaches only the post-checkpoint byte and end-offset
suffix required to reject that candidate, rebuilds its non-owning membership
index in place, and gives the existing prefix directly to candidate execution.
Rejection removes candidate-local additions, rejoins that bounded suffix, and
moves the restored owner back. Acceptance keeps the candidate owner and drops
the obsolete suffix. Direct rollback truncates to the same validated frontier.

This preserves the existing membership, counter, capacity, operation-depth,
main-memory, suspension, rollback, rejection, acceptance, and retirement
semantics. It adds no alternate path, cache, payload threshold, unsafe code,
or profiling-probe optimization.

## Focused exact gate

The exact baseline was commit `6d6b0c1c3ab9815818c6bc049baac42f8ae8cdcc`.
Both release binaries retained 32,768 distinct spellings and performed 4,096
production runtime-checkpoint captures under one `cycles:u,instructions:u`
`perf stat` execution and the authenticated `.132` public-copy interposer.
The selected baseline owner has three dense lanes in this larger fixture and
copies 1,507,328 bytes per capture.

| Counter                                    |                Baseline |                Final |                    Delta |
| ------------------------------------------ | ----------------------: | -------------------: | -----------------------: |
| Selected checkpoint `memcpy` calls / bytes |  12,288 / 6,174,015,488 |                0 / 0 | -12,288 / -6,174,015,488 |
| Whole-process `memcpy` calls / bytes       | 176,235 / 6,241,462,652 | 163,941 / 65,481,263 | -12,294 / -6,175,981,389 |
| Whole-process `memmove` calls / bytes      |                   2 / 0 |                2 / 0 |                    0 / 0 |
| Scoped allocations / requested bytes       |  12,302 / 6,183,248,896 |       14 / 9,233,408 | -12,288 / -6,174,015,488 |
| User cycles                                |           1,246,552,795 |           91,127,170 | -1,155,425,625 (-92.69%) |
| User instructions                          |             137,384,634 |          128,254,097 |      -9,130,537 (-6.65%) |
| Internal nanoseconds                       |             545,700,344 |           29,791,787 |   -515,908,557 (-94.54%) |
| Nanoseconds per checkpoint                 |                 133,227 |                7,273 |       -125,954 (-94.54%) |

The exact selected call and byte reductions equal the scoped allocation
reductions. Both reports reconcile, both retain two zero-byte `memmove` calls,
and both have zero overflow or probe-internal calls. The final report contains
no `RecycledStringPool`, `EngineUsageRuntime::clone`, or replacement owner.

## Remaining owners and validation

The `.132` table still assigns 2,333,768 `memcpy` calls and 392,073,024 bytes
to active dense node-lineage release. The active line-break node-view and page
placement rows dominate `memmove`. Outside those lanes, `.134` owns the
112,132,392-byte external-resource chain; TFM ligature pending-character
movement remains 112,216/18,852,288 plus two 82,348/13,834,464 `memcpy` rows;
and `CandidateRun::run` is the largest independent remaining `memmove` row at
122 calls and 1,424,838 bytes.

The complete `tex-state` and `tex-exec` owning/related suites pass: 560 + 12 +
1 and 759 + 4 + 24 tests respectively, with the two declared executor tests
ignored. Focused tests cover direct rollback, candidate rejection, suffix
membership restoration, and the pre-existing checkpoint lifecycle matrix.
`scripts/check.sh` passes all four standard gates: dprint, Biome, rustfmt, and
both declared clippy resolutions across 32 workspace members.

Ignored measurement evidence is under `target/umber2-66p0.8.40.133/`.
Baseline/final binary SHA-256 values are
`a4a567a19e8941f7ac04feaaa8c1aa4fe2ae999991075feb58cd88ccbe0139a3`
and
`21cc175fc3eab0ad84bc98f49318e9556982b9aba46bb0259f51cd8571ee3ce4`.
Baseline/final symbolized copy reports are
`f7a55a9336805c6d1b4c3654f53e4bb655917a6d493d50079ec78ba433bd6bf3`
and
`675990dbcf5db45c280ba4758745b9324d9458b6605ce34cf8e2069eb2a93dcc`;
counter receipts are
`7b6142a5db9580f038482099fc2cf8e11e061afd0cade38c72440ea1bf45a1b1`
and
`51207b518ea93e42deb6ac6239c5394a44d8bdba456c1cda530c349469a0e364`.
