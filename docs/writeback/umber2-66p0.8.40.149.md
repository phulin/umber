# `umber2-66p0.8.40.149`: post-rollback-lane integrated profile

## Authenticated bounded capture

Exactly one current-tree arXiv execution entered the engine at commit
`b4ded2e670315b265a6e4e6eacfa6312b4bd8de4` (tree
`5e9be101e57c293690963e8af61ad1885022da9a`). The Rust 1.93.0 profiling
binary has SHA-256
`3ffcb8338ae036d6927a59a01cb8389d0ab9fc84243c6f19122284bad7f0809b`,
ELF build ID `fc8699e87d635f10cdf8fd3444be1a316a717cb2`, and size
421,326,872 bytes. The checked public-copy interposer has SHA-256
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
the exact vector
`(50000000,49903532,9457781,15936698,35326903,4203)` for fuel charges,
token-frame steps, expanded deliveries, meaning lookups, scanner tokens, and
write expansions. Raw source, stored-token, macro-argument, and synthetic-
end-v deliveries were exactly `463672`, `30199338`, `19240431`, and `91`,
summing to `49903532`. Standard output was empty and the fuel endpoint
published no PDF or input receipt.

The capture contains 9,498 samples, reports zero lost samples, and sums to
approximately 113,897,588,036 sampled cycles. The simultaneous counter row
records 114,167,500,042 user cycles and 115,658,180,869 user instructions
(1.01 instructions/cycle) over 49.129782132 seconds. The outer guard observed
49.34 seconds wall, 54.13 user, 5.03 system, and 692,464 KiB peak RSS. These
are attributed, probed measurements rather than an uninstrumented latency
claim.

## Disjoint CPU owners and node separation

Self percentages below are disjoint sampled owners. Inclusive percentages are
overlapping ancestry and must not be added to each other or to self.

| Owner                                     | Inclusive |  Self |
| ----------------------------------------- | --------: | ----: |
| `advance_resident_command_into`           |     6.27% | 4.51% |
| `expand_classified_into`                  |    10.21% | 1.20% |
| `raw_delivery_entry`                      |     4.94% | 0.77% |
| `expanded_delivery_entry`                 |    10.12% | 0.66% |
| `ExecutionScratch::append_argument_token` |     0.67% | 0.53% |
| `scan_toks_buffers`                       |     3.66% | 0.44% |
| leading `Universe::with_command_context`  |     7.40% | 0.44% |
| `InputStack::push_row`                    |     0.22% | 0.20% |
| `InputState::render_context_for_levels`   |     0.65% | 0.15% |

The copy probe accounts separately for 8.28% self in `record_copy` and 0.83%
in its `memcpy` wrapper. The profiling allocator accounts for 0.86%, hot-core
measurement for 0.21%, and detailed raw-delivery accounting for 0.18%. The
`expand_classified_into` annotation also places profiling-only opcode atomics
inside that symbol, so its full 1.20% is not a production-removable ceiling.

The dominant 31.44% `ForkArena::payload_reservation_target`, 17.74%
`DenseBlockPayload::truncate`, and the node traversal/codec owners are not
command targets. This assigned base precedes integrated commit `39fda00bd`,
which removed the already-demonstrated by-value traversal family under
`.113.5.7`. The separately retained 692 MiB node/annex superblock high water is
owned by `umber2-klu1`. Neither family is rediscovered or ranked here.

The two exact public-copy leaders on this base are the known
`PageMaterialArena::span_chunk_node` row at 32,595,044 calls / 5,410,777,304
bytes and `append_reencoded_chunk_range` at 32,273,344 / 5,389,648,448. They
sum to 68.65% of all `memcpy` bytes and explain why this capture's CPU
denominator is unlike a post-`39fda00bd` row.

## Comparable deltas and exact censuses

The latest command-path CPU authority is `.145` at 20 million fuel. The
selected symbols moved as follows after `.146`--`.148`:

| Owner                                     | `.145` inclusive / self | Current inclusive / self |
| ----------------------------------------- | ----------------------: | -----------------------: |
| `advance_resident_command_into`           |         17.06% / 10.43% |            6.27% / 4.51% |
| `raw_delivery_entry`                      |          17.01% / 3.53% |            4.94% / 0.77% |
| `expanded_delivery_entry`                 |          26.41% / 1.48% |           10.12% / 0.66% |
| `scan_toks_buffers`                       |          15.07% / 1.50% |            3.66% / 0.44% |
| `ExecutionScratch::append_argument_token` |           1.93% / 1.55% |            0.67% / 0.53% |

This verifies that the three former selection targets no longer dominate the
sample ranking. It is not a causal percentage or speed comparison: `.145`
stopped at 20 million fuel, whereas this row reaches the later node-heavy 50
million endpoint. The exact focused evidence remains authoritative for the
three changes: `.146` and `.147` reduced code size without an instruction
win, while `.148` reduced the mixed-input instruction count by 1.94%.

The closest exact-vector structural authority is `.113.5.5`. Named allocation
calls fell from 7,596,254 to 7,596,240 and requested bytes from
28,389,785,396 to 28,389,753,012. The entire exact delta, -14 calls / -32,384
bytes, is in `delivery_and_scan`; every other owner is unchanged:

| Allocation owner           |         Calls |    Requested bytes |
| -------------------------- | ------------: | -----------------: |
| `delivery_and_scan`        |       414,684 |      8,885,236,724 |
| `semantic_apply`           |     6,998,144 |      2,460,283,921 |
| `evidence_publication`     |         3,670 |          1,569,635 |
| `cold_materialization`     |       179,077 |     17,040,994,012 |
| `attempt_scratch`          |           665 |          1,668,720 |
| all remaining named owners |             0 |                  0 |
| **Exact total**            | **7,596,240** | **28,389,753,012** |

Public-copy attribution also reconciles exactly, with zero overflow or
probe-internal calls:

| API       |       Current calls / bytes |    `.113.5.5` calls / bytes |       Delta calls / bytes |
| --------- | --------------------------: | --------------------------: | ------------------------: |
| `memcpy`  | 94,793,372 / 15,733,065,941 | 92,910,194 / 15,311,651,310 | +1,883,178 / +421,414,631 |
| `memmove` |          13,974 / 2,671,470 |          13,974 / 2,677,614 |                0 / -6,144 |
| Joint     | 94,807,346 / 15,735,737,411 | 92,924,168 / 15,314,328,924 | +1,883,178 / +421,408,487 |

The current report attributes no public copy to resident command advancement.
Peak RSS is effectively unchanged from `.113.5.5`: 692,464 versus 692,640 KiB
(-176 KiB). Copy and RSS movement therefore supplies no fourth command target.

## Exactly three ranked non-node targets

1. **Make the input row own its rollback epoch instead of indexing a side
   lane.** Repack the common row header so the eight-byte epoch/state stamp
   lives with its authoritative `InputLevel`, then delete
   `rollback_markers: Vec<RowRollbackMarker>`. Admission, advance, replacement,
   pop, and rollback should index one row and one owner, without increasing the
   88-byte row or introducing a scan. This targets the 4.51% self / 6.27%
   inclusive resident owner plus `InputStack::push_row` at 0.20% self. `.148`
   collapsed three parallel vectors into this one lane; it did not eliminate
   the surviving row/side-lane double indexing.
2. **Replace job-global expansion counting with delivery-local expansion
   state.** Delete `ExpansionState::cumulative_expansions` and its
   `CommandTimeline::record_cumulative_expansions` scalar journal. The expanded
   destination already needs only whether its current delivery expanded; keep
   that one bit local and retain it only in a genuinely suspended continuation.
   This removes a root field, snapshot/hash state, and a journal check from
   every expansion under the 1.20% self / 10.21% inclusive
   `expand_classified_into` owner. No existing Bead names this state.
3. **Construct rich expanded-delivery failures only on the cold edge.** Let the
   ordinary expanded destination loop return its command/end scalar directly
   and move `DeliveryErrorSlot`, cleanup, and resume-error translation into
   cold failure or genuine suspension helpers, deleting the successful-path
   `expanded_delivery_entry` wrapper. This targets its 0.66% disjoint self and
   10.12% inclusive envelope without adding a carrier, policy driver, or
   second loop. `.147` made this change only for `raw_delivery_entry`; the
   expanded path deliberately retained the eager wrapper.

The ranking uses current disjoint self evidence, discounts profiling-only
work, and uses inclusive ancestry only to define scope. Searches of completed,
open, and in-progress delivery/input issues found no owner for the latter two
targets; `.148` is the explicit predecessor of the first. Node work,
instrumentation, parity, and output are excluded.

## Evidence

Ignored issue-private evidence is under `target/umber2-66p0.8.40.149/evidence/`.
SHA-256 values for `perf.data`, raw copy data, symbolized copy report, self
report, inclusive report, counter receipt, engine stderr, and outer timing are
respectively
`5ee4b514c3c01aa4ea3f16ab1edc6fe40a2c283ec76d4244e8b96a7a8b4364c5`,
`73984176481cbbb5cf6ce53969479efd08cf668df2aef11fa4c53d227ea100db`,
`37730b493b9d53b9e6ad0d28e47373911bd7995d90e1e3df0f330dd4fc14d9c9`,
`de1ce6fffec92122ca7de0054b3a7c190df89313fb1a2dc51507ba0f73f36e1c`,
`84005a1bb0c8f75876b548b9fc7d84679121f46051f047374a0706dca8bcdbba`,
`a0335c113ab78f4ff06d486abf1d491bcddb8c57e4c088c3e6e8edd8eb03c159`,
`4f47bbba7dbf202f3c4a6c5e9bf35f8e2647ea08bd904f7385fb1bce8201b1ca`,
and
`a4f69be768775228061e1c2492576d6c7a9c19943ddec5bba14816ce9107c456`.
