# `umber2-66p0.8.40.129`: derive ordinary delivery freshness from resident input

## Selection authority

The integrated `.127` authenticated 20,000,000-command capture remains the
sole broad authority. At exact work vector
`(20000000,19907047,2216876,6018541,16781945,4011)`, `raw_delivery_entry`
ranked at 3.82% application self after `.127` removed the duplicate profiling
census and `.128` removed the universal resident-top carrier. No new corpus
execution or broad profile was run.

The remaining entry published every delivered command's 16-byte
input-level/position stamp into `CommandProcessor`, then uncommon backup,
alignment handoff, retirement, and suspension paths compared their command
with that mirror. The authoritative stored-token, macro-body, and
macro-argument rows already retain the same level identity and their cursor
exactly one position after a successful delivery. Mirroring the coordinate on
every ordinary transition was not TeX semantics, provenance, input ownership,
or observation order.

## Architectural simplification

Ordinary stored and macro delivery now publishes only an episode-local
availability bit. A consumer that actually needs immediate freshness compares
the command's delivery coordinate with the authoritative top row and its
resident cursor. A fresh processor starts with the bit clear, a later raw
request clears it, and consumption clears it, so a command from another
episode or an earlier delivery remains stale even when a coordinate is
rewound.

One explicit coordinate remains for cases with no derivable resident
predecessor: direct-source delivery uses a physical position rather than the
row's logical cursor, synthetic `endv` has already retired its supplying row,
and a genuine typed suspension explicitly readmits its settled command in a
fresh processor episode. These exceptional paths construct that proof only
when required. Observation sequencing remains separate.

This removes one full-coordinate publication from the default raw-delivery
path. It adds no alternate route, threshold, cache lookup, or special-case
semantic fast path. Raw token meaning, source provenance, input ownership,
backup, suspension, rollback, retirement, acceptance, alignment correction,
and delivery observation are unchanged.

## Focused before/after gate

The exact baseline is `.128`'s accepted production-profiling
`mixed_macro_resident_pipeline` binary. The final binary ran the same row once
under `perf stat` and once under the checked public-copy interposer. Both
report 2,000,000 macro-body transitions, 1,000,000 parameter deliveries,
1,000,004 replay words, 2,000,004 raw frame steps, 1,000,000 expanded
deliveries, 1,000,001 macro expansions, zero suspension moves, zero command
copies, and zero warmed allocations or requested bytes.

| Counter                              |      Baseline |         Final |                  Delta |
| ------------------------------------ | ------------: | ------------: | ---------------------: |
| User instructions                    | 2,434,755,137 | 2,413,740,027 |   -21,015,110 (-0.86%) |
| User cycles                          |   975,947,863 | 1,723,694,629 | +747,746,766 (+76.62%) |
| Internal elapsed nanoseconds         |   379,034,930 |   663,265,540 | +284,230,610 (+74.99%) |
| Warmed allocations / requested bytes |         0 / 0 |         0 / 0 |                  0 / 0 |
| Public `memcpy` calls / bytes        | 130 / 344,169 | 132 / 344,535 |              +2 / +366 |
| Public `memmove` calls / bytes       |         2 / 0 |         2 / 0 |                  0 / 0 |

The exact instruction reduction is the primary CPU result: about 10.5
instructions per macro-body transition. Cycles and elapsed time were dominated
by concurrent host load and are not used as acceptance evidence. Exact public-
copy attribution reconciles both APIs with zero collision overflow or probe-
internal calls. No copy is attributed to raw delivery or another `tex-command`
hot function; the two-call, 366-byte process-total increase is initialization
layout outside the warmed zero-allocation row.

## Evidence and validation

Ignored evidence is under `target/umber2-66p0.8.40.129/focused-gate/`. Baseline
and final binary SHA-256 values are
`13787725294efd58d0cdd2ff6ae57ba8eb1be9b98d3af91148baf8a072ff2f04` and
`b4de9369bf5ff8a1ab18195c7e84cb3a634a1fad37127aa7dfbe0110a7251e0f`.
Their `perf stat` receipts are
`03fd99ed3b17957f563a08befaacbf8dde4b3b467b4c814dfb37dfcd893bde59` and
`d1b52603cc1f0d401306eb2a237fdfffbdde6423e7e5d29857b96a2800d6436c`;
their symbolized copy reports are
`2a69c2ea6825d7d648740e32c228cc3d409c0d94031f5010def089ccb08b3630`
and
`1774d2a2b1913fb30278306c7d694cb088a47893c550414bdde2697917963c6d`.
The checked interposer SHA-256 remains
`8afd6ca34d91c28fccbccff4273a2f47dfa61861d3bceb53cf2c14dc7d2be16e`.

`cargo test -q --tests -p tex-command` passes 384 unit and 23 boundary tests.
The complete `cargo test -q --tests` routine suite passes from a fresh
issue-local target after interrupted prior builds left the checkout's shared
incremental cache with missing LLVM objects. `scripts/check.sh` passes all four
gates: dprint, biome, rustfmt, and both clippy resolutions.
