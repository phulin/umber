# `umber2-66p0.8.40.153`: post-local-state integrated command profile

## Authenticated bounded capture

Exactly one current-tree arXiv execution entered the engine at commit
`8532494f51feecef26af7d0907dae5944a787858` (tree
`bcd7531447b98aa3b04629580a2955305a97778f`). The Rust 1.93.0 profiling
binary has SHA-256
`f2596014645f070f5e6bbe6d32c67c700f11dc3f7d5bbdb57381b6d78e1028ca`,
ELF build ID `434426c45acee5fb2f1c98a5f0474106718ec08c`, and size
422,442,528 bytes. The checked public-copy interposer has SHA-256
`8afd6ca34d91c28fccbccff4273a2f47dfa61861d3bceb53cf2c14dc7d2be16e`.
The capture used 199 Hz `cycles:u`, 8,192-byte DWARF callchains, and an 8 MiB
ring.

The workload remained arXiv `2606.12566`: `ArXiv.tex` SHA-256
`816440f61d611fa57cef802e6f372b9337beef1cc4e48e5536d4bad1014ec537`,
fixed source epoch `1787080434`, schema-12 format object
`ahash64-v1-2b924b5bba05d8a0` with SHA-256
`ddd082db722654fd9c47e1080d8e128d1a5b0cfb3186f9e3bc01f8943f559ad4`,
and the preserved 2026-03-01 distribution manifest with SHA-256
`a68acebc1a83fd4ec0ce8c3baed4e8fe01de9b37e5a878b2f1fc203c2f20662f`
and aHash64 `df66c327ae636145`. The ordered 123-key closure has SHA-256
`e4f4113c9057af88c239d40d3041f598871a0a7a895f8bd63f89d7c77682ab7e`.
No distribution, format, or shared cache was regenerated or purged.

The guards were 50,000,000 canonical-command fuel, 100,000,000 executor
steps, 90 seconds, and 1,536 MiB aggregate RSS. Expected status 1 occurred at
the exact vector `(50000000,49903532,9457781,15936698,35326903,4203)` for
fuel charges, token-frame steps, expanded deliveries, meaning lookups, scanner
tokens, and write expansions. Raw source, stored-token, macro-argument, and
synthetic-end-v deliveries were exactly `463672`, `30199338`, `19240431`, and
`91`, summing to `49903532`. Standard output was empty and the fuel endpoint
published no PDF or input receipt.

The capture contains 8,797 samples, reports zero lost samples, and sums to
approximately 105,061,654,049 sampled cycles. The simultaneous counter row
records 105,335,209,814 user cycles and 86,497,658,833 user instructions (0.82
instructions/cycle) over 49.132326311 seconds. The outer guard observed 49.36
seconds wall, 50.23 user, 5.32 system, and 692,396 KiB peak RSS. These are
attributed, probed measurements rather than an uninstrumented latency claim.

## CPU attribution and excluded node ownership

Self percentages are disjoint sampled owners. Inclusive percentages overlap
through ancestry and must not be added.

| Non-node owner                            | Inclusive |  Self |
| ----------------------------------------- | --------: | ----: |
| `advance_resident_command_into`           |     6.94% | 4.74% |
| `expand_classified_into`                  |    12.29% | 1.50% |
| `raw_delivery_entry`                      |     5.83% | 1.09% |
| `ExecutionScratch::append_argument_token` |     0.86% | 0.70% |
| `expanded_delivery_entry`                 |    11.48% | 0.60% |
| `scan_toks_buffers`                       |     4.46% | 0.45% |
| leading `Universe::with_command_context`  |     9.07% | 0.34% |
| `InputState::render_context_for_levels`   |     0.95% | 0.18% |

The copy probe accounts for 1.17% self in `record_copy` and 0.09% in its
`memcpy` wrapper. The profiling allocator accounts for 0.40%, raw-delivery
accounting for 0.33%, phase accounting for 0.22%, and command-family accounting
for 0.14%. Profiling-only opcode atomics are also inside
`expand_classified_into`, so its complete 1.50% is not a production-removable
ceiling.

The dominant `ForkArena::payload_reservation_target` at 36.47% inclusive /
36.27% self, `DenseBlockPayload::truncate` at 19.17% / 19.17%, and their
`copy_record_chunk_prefix`, region-copy, durable-box, page-history, retirement,
and node-codec ancestry are one node ownership family. The integrated borrowed
compact-node traversal removed its former by-value public-copy storm, but it
did not reduce the simultaneously live region high water or RSS. That remaining
ownership and retirement problem is already active as
`umber2-66p0.8.40.113.5.8`; it is excluded from the targets below rather than
rediscovered here.

## Exact census and comparison with `.149`

The work vector and all four raw-delivery subtotals are byte-for-byte equal to
the compatible `.149` capture. The exact allocation census is:

| Allocation owner           |         Calls |    Requested bytes |
| -------------------------- | ------------: | -----------------: |
| `delivery_and_scan`        |       414,677 |      8,885,220,532 |
| `semantic_apply`           |     2,765,037 |      1,139,068,411 |
| `evidence_publication`     |         3,670 |          1,569,635 |
| `cold_materialization`     |       179,074 |     17,040,993,116 |
| `attempt_scratch`          |           665 |          1,668,720 |
| all remaining named owners |             0 |                  0 |
| **Exact total**            | **3,363,123** | **27,068,520,414** |

Against `.149`, calls fell by 4,233,117 and requested bytes by 1,321,232,598.
The integrated node traversal accounts for 4,233,107 calls and 1,321,215,510
bytes of that delta in `semantic_apply`. The combined `.150`--`.152` tree is
therefore comparable only at the remaining exact owner deltas:
`delivery_and_scan` is lower by 7 calls / 16,192 bytes and
`cold_materialization` by 3 calls / 896 bytes; every other non-node owner is
unchanged.

Public-copy attribution reconciles exactly with zero overflow or
probe-internal calls:

| API       |     Current calls / bytes |        `.149` calls / bytes |           Delta calls / bytes |
| --------- | ------------------------: | --------------------------: | ----------------------------: |
| `memcpy`  | 9,584,526 / 1,476,059,455 | 94,793,372 / 15,733,065,941 | -85,208,846 / -14,257,006,486 |
| `memmove` |        13,974 / 2,666,350 |          13,974 / 2,671,470 |                    0 / -5,120 |
| Joint     | 9,598,500 / 1,478,725,805 | 94,807,346 / 15,735,737,411 | -85,208,846 / -14,257,011,606 |

The 90.62% `memcpy`-byte reduction is the integrated compact-node traversal
fix, not a local-state claim. Peak RSS is effectively unchanged at 692,396
versus 692,464 KiB (-68 KiB), consistent with the separately active live-node
ownership issue.

The simultaneous counter row is 7.74% lower in cycles and 25.21% lower in
instructions than `.149`, while probed wall time is effectively unchanged.
That aggregate includes the traversal fix and changed sampled/probe cost, so it
does not isolate `.150`--`.152`. Their focused receipts remain the causal
evidence: `.150` removed 0.34% instructions from its mixed-input row, `.151`
removed 0.69% from its expansion row, and `.152` removed 0.09% plus 62.45% of
the selected expanded-entry code. In this integrated profile,
`expanded_delivery_entry` self attribution falls from 0.66% to 0.60%; other
command percentages rise against the much smaller node/copy denominator and
must not be reported as regressions.

## Exactly three ranked non-node targets

1. **Give all token-backed input rows one resident header and transition.** Move
   the shared frame, packed rollback marker, behavior, cursor advance,
   parameter interception, and exhaustion facts for replay, attempt, and
   durable storage into one authoritative token-row header; dispatch only the
   storage-specific word read, then enter the already-shared admission tail.
   This deletes three replicated first-touch/advance/parameter state machines
   under the 4.74% self / 6.94% inclusive resident owner. It is distinct from
   `.128`'s deleted universal top carrier, `.146`'s shared final admission tail,
   and `.150`'s row-owned rollback marker: all three concrete token arms remain
   duplicated in current source. Macro-body, macro-argument, and source rows
   retain their genuinely different lifetimes; no cache or command-family path
   is added.
2. **Make the delivery destination permanently occupied on the hot path.** Let
   raw and expanded entry own one initialized `CurrentCommand` slot throughout
   ordinary fetch/classify/return, and move it out only when a real suspension
   or cold failure needs ownership. Delete the repeated `Option<CurrentCommand>`
   vacancy tests, placeholder reinstallation, `as_ref`/`as_mut` recovery, and
   success-path `take` protocol visible in both loops. This targets
   `raw_delivery_entry` at 1.09% self / 5.83% inclusive and
   `expanded_delivery_entry` at 0.60% / 11.48%. `.123` removed semantic work
   from placeholder construction and `.147`/`.152` made rich errors cold, but
   none removed this surviving destination-state layer. No policy driver,
   second representation, or special command path is required.
3. **Make command-visible state one directly borrowable resident owner.** Group
   the fields currently reassembled into the broad `CommandContext` reference
   facade under one Universe subobject and lend that owner directly across the
   existing processor/application episode. This deletes repeated facade
   construction and field-by-field lookup at the remaining
   `Universe::with_command_context` entries, whose leading monomorph is 0.34%
   self / 9.07% inclusive and whose additional monomorphs contribute visible
   0.21% and 0.20% self rows. `.117` removed a copied 240-byte return value and
   `.142` removed one residual cold reconstruction; neither changed the
   repeated construction boundary that remains. Existing borrow lifetimes and
   tracked admission stay authoritative; no persistent alias, cache, or new
   state is introduced.

The ranking uses disjoint current self evidence, discounts profiling-only work,
and uses inclusive ancestry only to define scope. Searches of open and
in-progress Beads found no owner for these three residual boundaries. Node,
page-history, instrumentation, diagnostic, parity, and output work are
excluded.

## Evidence

Ignored issue-private evidence is under `target/umber2-66p0.8.40.153/evidence/`.
SHA-256 values for `perf.data`, raw copy data, symbolized copy report, self
report, inclusive report, counter receipt, engine stderr, and outer timing are
respectively
`7bcaa23b17e85dd6df5416b4a7a89ba71000aa9fd26cf4be16122bf4fa5c252d`,
`fe5b432ace367cbe0f66b59214522bae1e4a1d6b2bc799a69a4f275c8b0ffec9`,
`10fba7763f701448d3ebe37c767cbf7d73b9bb9c59ff7a746ec51e0968db0ca2`,
`89ae42624c48eaf538c4c8bd264d0c22002c58e1689ce18999e092a39161bbb6`,
`f907e870068afa5da93564c1bd25e4bac1acde63cd584b679779c2d3927e07da`,
`6c809a29c03debbe47841c0f9818a94e2a9074747c89ad43d17a8647f15e832b`,
`e4c429eef45dad9e1b5ed7261001702ce7543f5d270ea0bfffd2d97163234e69`,
and
`3d3d2db155f1616fb88cb612c7459888564363ec3054d10b96999d2c60305e6a`.
