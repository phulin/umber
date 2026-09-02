# `umber2-66p0.8.40.145`: post-input-simplification CPU profile

## Authenticated bounded capture

Exactly one current-tree arXiv execution entered the engine at commit
`789b8f67a0b679e2661b1c064bf763cec2973dd5` (tree
`ba3583d7500e5503f16f7b0b473128123d7f5cdd`). The profiling binary has
SHA-256
`93876f84be41a7a4501758dd7f367359fa2a1aaef545f96596b34a40e28302c0`,
build ID `fef8c9997ede41286a6ffd1bb3b3779d03d77f3d`, and size 418,289,520
bytes. The checked public-copy interposer has SHA-256
`8afd6ca34d91c28fccbccff4273a2f47dfa61861d3bceb53cf2c14dc7d2be16e`.
The capture used 199 Hz `cycles:u`, 8,192-byte DWARF callchains, and an 8 MiB
ring.

The workload was arXiv `2606.12566`: `ArXiv.tex` SHA-256
`816440f61d611fa57cef802e6f372b9337beef1cc4e48e5536d4bad1014ec537`,
fixed source epoch `1787080434`, schema-12 format object
`ahash64-v1-2b924b5bba05d8a0` with SHA-256
`ddd082db722654fd9c47e1080d8e128d1a5b0cfb3186f9e3bc01f8943f559ad4`,
and distribution manifest SHA-256
`a68acebc1a83fd4ec0ce8c3baed4e8fe01de9b37e5a878b2f1fc203c2f20662f`
with aHash64 `df66c327ae636145`. The ordered 123-key closure has SHA-256
`e4f4113c9057af88c239d40d3041f598871a0a7a895f8bd63f89d7c77682ab7e`.
The shared distribution directory was replaced by another process six minutes
after capture; its captured manifest and format object remain byte-identical
under `target/texlive-snapshot.stale-a68acebc`, and the raw command receipt
retains the original path. No format or distribution was generated here.

The guards were 20,000,000 canonical command fuel, 40,000,000 executor steps,
45 seconds, and 1,536 MiB RSS. Expected status 1 occurred at the exact `.140`
work vector `(20000000,19907047,2216877,6018482,16781922,4011)`. Raw source,
stored-token, macro-argument, and synthetic-end-v deliveries were exactly
`463168`, `11520891`, `7922897`, and `91`, summing to `19907047`. The capture
contains 2,578 samples, zero lost samples, and approximately 29,610,244,657
cycles. The wrapper observed 18.55 seconds wall, 14.64 user, 2.36 system, and
162,936 KiB peak RSS, but it also populated a private cold resource cache;
neither those times nor the different sample count support a total-speed claim.

## Disjoint CPU owners and overlapping ancestry

Self percentages below are disjoint sampled owners. Inclusive percentages are
overlapping ancestry and must not be added to each other or to self.

| Owner                                     | Inclusive |   Self | `.140` inclusive / self |
| ----------------------------------------- | --------: | -----: | ----------------------: |
| `expand_classified_into`                  |    32.58% |  4.52% |          32.87% / 3.92% |
| `execute_direct_episode`                  |    29.74% |  0.70% |          27.60% / 0.53% |
| `expanded_delivery_entry`                 |    26.41% |  1.48% |          27.23% / 1.53% |
| `Universe::with_command_context`          |    18.61% |  1.12% |          18.36% / 0.38% |
| `advance_resident_command_into`           |    17.06% | 10.43% |         21.54% / 13.27% |
| `raw_delivery_entry`                      |    17.01% |  3.53% |          19.92% / 2.67% |
| `scan_toks_buffers`                       |    15.07% |  1.50% |          16.27% / 0.99% |
| `ExecutionScratch::append_argument_token` |     1.93% |  1.55% |           1.95% / 1.56% |
| `next_compact_exact_byte_step`            |     2.19% |  0.65% |           2.60% / 0.93% |

Resident input advancement therefore moved down materially in sampled
ownership: 13.27% to 10.43% self (-2.84 percentage points, -21.4% relative)
and 21.54% to 17.06% inclusive (-4.48 points, -20.8% relative). It remains the
largest concrete non-probe, non-node self owner. The byte tokenizer likewise
moved from 0.93% to 0.65% self. Expansion, raw-entry, and scanner self moved by
+0.60, +0.86, and +0.51 points; these are owner movements in one sampled run,
not regressions or isolated causal claims. In particular, this issue ran no
commit A/B series.

The copy probe itself contributed 3.11% self plus 0.64% in its `memcpy`
wrapper, detailed fuel accounting 0.77%, and the profiling allocator 0.67%.
Node-arena/fork-arena owners, including the active 32-byte `NodeRecord`
cutover, are excluded from interpretation and recommendations.

## Exact public-copy and allocation census

The public-copy report reconciles exactly:

| API       |                 Current |                  `.140` |                Delta |
| --------- | ----------------------: | ----------------------: | -------------------: |
| `memcpy`  | 6,683,962 / 826,136,215 | 6,754,872 / 817,413,380 | -70,910 / +8,722,835 |
| `memmove` |    238,866 / 40,779,494 |    238,866 / 40,758,998 |          0 / +20,496 |
| **Joint** | 6,922,828 / 866,915,709 | 6,993,738 / 858,172,378 | -70,910 / +8,743,331 |

Counts precede requested bytes. Both APIs recorded zero overflow and
probe-internal calls. `memcpy` had 129,949 collision probes, maximum one;
`memmove` had 122, maximum one. The dominant
`ChunkStorage::release_lineage` row remains exactly 2,445,622 calls /
410,864,496 bytes and is excluded node work. The current report contains no
copy attributed to resident advancement. The `.142` 33,886-call /
8,132,640-byte `CommandContext` move and `.143` 56,964-call /
1,842,726-byte valid-line copy are absent, confirming zero current remainder.
The joint byte increase is distributed mainly among active node, line-break,
page, decompression, and resource-cache owners and is not attributed to the
input changes.

Named allocation scopes are disjoint innermost owners and sum to the exact
process census:

| Allocation owner                |   Calls | Requested bytes | Calls vs `.140` | Bytes vs `.140` |
| ------------------------------- | ------: | --------------: | --------------: | --------------: |
| `delivery_and_scan`             | 219,249 |   8,578,709,753 |         -25,826 |        -727,794 |
| `semantic_apply`                | 595,024 |     335,062,000 |            +199 |      +1,231,056 |
| `evidence_publication`          |   1,138 |         815,729 |               0 |               0 |
| `cold_materialization`          | 179,066 |  17,040,790,796 |               0 |        +110,696 |
| `attempt_scratch`               |     665 |       1,668,720 |               0 |               0 |
| all four remaining named owners |       0 |               0 |               0 |               0 |
| **Exact total**                 | 995,142 |  25,957,046,998 |         -25,627 |        +613,958 |

Thus the current census has 25,826 fewer delivery-and-scan allocation calls
than `.140` at the exact work vector, while requested bytes are essentially
flat.
Requested bytes are allocator requests, not peak resident memory, and the
cross-owner byte shifts are not a total-memory or speed claim.

## Exactly three ranked next targets

1. **Give all resident variants one command-admission tail.** Keep the already
   admitted replay, attempt, durable, macro-body, and macro-argument cursor
   operations, but let each arm produce only its word/origin/position/source
   scalars and enter one branch-independent `write_resolved_delivery` plus
   `settle_resident_delivery` tail. Do not add a carrier or another loop. This
   targets the 10.43% self / 17.06% inclusive dominant owner and its 10,985-byte
   profiling monomorph, where the stored-word macro and bespoke body/argument
   arms still replicate resolution and settlement. Completed `.112` admitted
   each storage domain once, `.124`--`.126` simplified cursor coordinates,
   `.128` removed the resident top carrier, and `.144` made argument advance
   direct; none consolidates this surviving common tail. This also preserves
   `.94.2`'s singular macro loop rather than reopening it.
2. **Make delivery failure state cold at the raw-entry boundary.** Let the
   ordinary raw loop return its hot command/end scalar through the existing
   caller slot, and construct `DeliveryErrorSlot`, rich `CommandError`, and the
   general `Result<DeliveryStatus, _>` protocol only after a cold transition.
   The 1,312-byte `raw_delivery_entry` is now 3.53% disjoint self, and its local
   annotation charges placeholder initialization, a large prologue, repeated
   destination tests, and final status storage. Completed `.103` deleted the
   generic delivery policy driver and `.123` deleted fake semantic packing
   inside the empty command slot; this target is the remaining per-call
   error/status transport, not either closed representation. `.129` and `.130`
   own freshness-coordinate and expanded-dispatch carriers, respectively.
3. **Collapse parallel resident-row rollback metadata into one compact stamp
   lane.** Replace the separate `touched`, `partially_captured`, and
   `cold_state_captured` vectors with one compact per-row epoch record and make
   the selected row perform its single first-touch check and cursor advance.
   This targets `InputStack::push_row` at 0.92% self plus the repeated epoch
   indexing inside the 10.43% resident owner, while retaining the documented
   one inverse per checkpoint interval. Completed `.67` established the
   semantic `InputStack`, and `.1` fused the former closure-based cursor
   mutation, but both leave these three parallel lanes in current source. No
   completed, active, or rejected Bead names or owns their consolidation.

The ranking is by likely CPU return from current disjoint self evidence;
inclusive ancestry is supporting scope only. Searches covered completed
input/delivery work `.1`, `.17`, `.24`, `.45`, `.67`, `.73`, `.94.2`,
`.103`--`.114`, and `.123`--`.144`, current open/in-progress work, and retired
or rejected command/input proposals. The active `.113.5.4` node-record cutover,
`.38` authority work, and all parity/format issues are deliberately absent.

## Evidence

Ignored issue-private evidence is under `target/umber2-66p0.8.40.145/`.
SHA-256 values for `perf.data`, raw copy data, symbolized copy report,
allocation census, identity receipt, self report, inclusive report, timing
receipt, and engine stderr are respectively
`f47ee673b020c33c6b8b50221afe65c8f6dc93501101bf5585fe87361a100681`,
`3f9fdf4dda483fa3dff2dd5e41d7dba318ffd388ffb4df6543b419da0f588317`,
`7edc2407a9cf105745174dffbccfd8dde342810be1ecc3ac40ed8dfbd00c4cb5`,
`241818e9d733de88ce4cf3c3621360216594c3b82d02e6d859d3edfaa99c7985`,
`b1776867bff395ea954da10207b8c307dcec21e6a133020cbfa3613415e89e45`,
`e949d20b4bd9981d7a6a20e85df05b5e764ad6f979bbc1674fca37f1c76ada87`,
`cb0ff4a7477da9d18cc2cdf38adc9255581b6d12ad3f4bc914f6e97cd7442cba`,
`6a91bbc8c69de23e6fa876bf97a7548fc6bdc4a14ba8257c891d2bc07730c06f`,
and
`5445f411d9dea22bbf39a258a62e2d2f72fa3d96f174f819ad670a0fb393d792`.
