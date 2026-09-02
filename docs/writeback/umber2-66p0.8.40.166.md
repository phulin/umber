# `umber2-66p0.8.40.166`: fixed-context integrated profile

## Authenticated bounded capture

Exactly one current-tree arXiv execution entered the engine at commit
`e0b158ef91954df4fb608900e482f9521ba5e38d` (tree
`4ed5b3294ac6e1955e82af868174c1ed3f0780fa`). The Rust 1.93.0 profiling
binary has SHA-256
`ff0a5f38f3b87890bf39c5d815c78a0c4dce43120234de65763b59d8296fd946`,
ELF build ID `f9308d89fc9f5b287b9a2370b825fd6e6d09a150`, and size 425,559,896
bytes. The checked public-copy interposer has SHA-256
`8afd6ca34d91c28fccbccff4273a2f47dfa61861d3bceb53cf2c14dc7d2be16e`.
The capture used 199 Hz `cycles:u`, 8,192-byte DWARF callchains, an 8 MiB
ring, and the ordinary 8,192 KiB native stack. No CPU hold, 100M execution,
second authority row, or measurement series ran.

The workload remained arXiv `2606.12566`: `ArXiv.tex` SHA-256
`816440f61d611fa57cef802e6f372b9337beef1cc4e48e5536d4bad1014ec537`,
fixed source epoch `1787080434`, schema-12 format object
`ahash64-v1-2b924b5bba05d8a0` with SHA-256
`ddd082db722654fd9c47e1080d8e128d1a5b0cfb3186f9e3bc01f8943f559ad4`,
and the preserved 2026-03-01 distribution manifest with SHA-256
`a68acebc1a83fd4ec0ce8c3baed4e8fe01de9b37e5a878b2f1fc203c2f20662f`
and aHash64 `df66c327ae636145`. The ordered 123-key closure has SHA-256
`e4f4113c9057af88c239d40d3041f598871a0a7a895f8bd63f89d7c77682ab7e`.
The offline run admitted 124 objects into its new issue-private native cache;
it did not regenerate or purge the distribution, format, or shared cache.

The guards were 50,000,000 canonical-command fuel, 100,000,000 executor
steps, 90 seconds, and 1,536 MiB aggregate RSS. Expected status 1 occurred at
the exact vector `(50000000,49903532,9457781,15936698,35326903,4203)` for
fuel charges, token-frame steps, expanded deliveries, meaning lookups, scanner
tokens, and write expansions. Raw source, stored-token, macro-argument, and
synthetic-end-v deliveries were exactly `463672`, `30199338`, `19240431`, and
`91`, summing to `49903532`. Standard output was empty and the fuel endpoint
published no PDF or input receipt.

The capture contains exactly 4,944 samples, reports zero lost samples, and its
sampled periods sum to 58,701,081,091 cycles. The simultaneous counter row
records 59,094,949,361 user cycles and 67,173,156,282 user instructions (1.14
instructions/cycle) over 30.222679050 seconds. The outer guard observed 30.55
seconds wall, 28.23 user, 3.13 system, and 165,940 KiB peak RSS. These are
attributed, probed measurements rather than an uninstrumented latency claim.

## Grouped self CPU and storage

Self percentages are disjoint sampled owners. Inclusive percentages overlap
through ancestry and must not be added.

| Owner or grouped family                           | Inclusive |   Self |
| ------------------------------------------------- | --------: | -----: |
| `advance_resident_command_into`                   |    16.03% | 11.05% |
| `expand_classified_occupied`                      |    27.11% |  3.09% |
| `raw_delivery_entry`                              |    13.26% |  2.98% |
| `ForkArena::admitted_chunk_value`                 |     2.41% |  2.41% |
| checked copy-probe family                         |         - |  2.20% |
| all `Universe::with_command_context` monomorphs   |         - |  1.93% |
| `expanded_delivery_entry`                         |    27.27% |  1.71% |
| `ForkArena::append_unsealed_list`                 |     1.75% |  1.49% |
| `PageMaterialArena::append_reencoded_chunk_range` |     4.32% |  1.29% |
| `ArenaListIter::next`                             |     1.28% |  1.28% |
| profiling allocator family                        |         - |  1.26% |
| `ExecutionScratch::append_argument_token`         |     1.58% |  1.22% |
| `scan_toks_buffers`                               |     9.27% |  1.10% |

The grouped context row is the exact sum of six disjoint self symbols (0.97%,
0.45%, 0.31%, 0.16%, 0.02%, and 0.02%); its leading monomorph is 21.53%
inclusive. The checked copy row is `record_copy` at 1.72%, its `memcpy`
wrapper at 0.46%, and probe TLS at 0.02%. The profiling allocator row groups
allocation, reallocation, scope, drop, and deallocation symbols. These grouped
instrumentation rows are not production-removable application ceilings.

The exact named allocation census is:

| Allocation owner           |         Calls |    Requested bytes |
| -------------------------- | ------------: | -----------------: |
| `delivery_and_scan`        |       403,042 |      8,790,477,716 |
| `semantic_apply`           |     2,720,133 |        578,462,529 |
| `evidence_publication`     |         3,670 |          1,569,635 |
| `cold_materialization`     |       179,074 |     17,040,745,972 |
| `attempt_scratch`          |           665 |          1,668,720 |
| all remaining named owners |             0 |                  0 |
| **Exact total**            | **3,306,584** | **26,412,924,572** |

The node pool again reached exactly 68 live node blocks / 4,456,448 payload
bytes and 80 live annex blocks / 5,242,880 payload bytes, ending with zero live
blocks. Node fresh/reuse/release totals were `68/1560/1622`; annex totals were
`80/36962/37031`. The largest sampled owner state held 22 current-page plus
four durable/other node blocks and 66 current-page plus nine durable/other
annex blocks. Across 873 page installations, 872 were zero-copy takes and one
was an on-demand promotion.

## Exact public-copy result

The report reconciles every caller with zero overflow or probe-internal calls:

| API       |   Current calls / bytes |      `.157` calls / bytes |       Delta calls / bytes |
| --------- | ----------------------: | ------------------------: | ------------------------: |
| `memcpy`  | 6,931,009 / 848,898,109 | 9,203,569 / 1,245,500,251 | -2,272,560 / -396,602,142 |
| `memmove` |      21,482 / 2,799,870 |        21,610 / 2,827,518 |            -128 / -27,648 |
| Joint     | 6,952,491 / 851,697,979 | 9,225,179 / 1,248,327,769 | -2,272,688 / -396,629,790 |

The leading exact `memcpy` caller bins are:

| Symbolized caller                                               |   Calls |      Bytes |
| --------------------------------------------------------------- | ------: | ---------: |
| `NodeCursor::try_for_each_range` option projection (`0xdd9d6b`) | 122,706 | 20,491,902 |
| `NodeCursor::try_for_each_range` nested closure (`0xdd9da4`)    | 122,706 | 20,491,902 |
| second range monomorph option projection (`0x1362339`)          | 122,706 | 20,491,902 |
| second range monomorph callback (`0x1362366`)                   | 122,706 | 20,491,902 |
| `warn_cross_file_group_close` (`0xf6a865`)                      |  46,616 | 19,019,328 |

The leading exact `memmove` caller bins are:

| Symbolized caller                    | Calls |     Bytes |
| ------------------------------------ | ----: | --------: |
| `CandidateRun::run` (`0x1084f95`)    |   128 | 1,493,888 |
| `BTree` leaf insertion (`0x12e2ae0`) | 3,158 |   313,704 |
| `BTree` leaf insertion (`0x1579595`) |   679 |    95,456 |
| `BTree` leaf insertion (`0x15795f2`) |   679 |    71,592 |
| `BTree` leaf insertion (`0x15825b0`) | 3,284 |    57,736 |

The complete symbolized report retains the top 40 exact caller bins for each
API and the raw report retains every bin.

## Comparison with `.157`

The semantic vector, all four raw-delivery subtotals, node/annex block peaks,
and page-installation census are byte-for-byte equal to `.157`.

| Measure                   |         `.157` |        Current |                    Change |
| ------------------------- | -------------: | -------------: | ------------------------: |
| Outer wall                |        19.32 s |        30.55 s |        +11.23 s (+58.13%) |
| Outer user                |        19.54 s |        28.23 s |         +8.69 s (+44.47%) |
| Outer system              |         2.19 s |         3.13 s |         +0.94 s (+42.92%) |
| Simultaneous user cycles  | 40,323,285,089 | 59,094,949,361 | +18,771,664,272 (+46.55%) |
| Simultaneous instructions | 71,563,607,021 | 67,173,156,282 |   -4,390,450,739 (-6.14%) |
| Peak RSS                  |    166,820 KiB |    165,940 KiB |         -880 KiB (-0.53%) |
| Named allocation calls    |      3,346,017 |      3,306,584 |          -39,433 (-1.18%) |
| Named requested bytes     | 26,510,056,981 | 26,412,924,572 |      -97,132,409 (-0.37%) |
| Joint public-copy calls   |      9,225,179 |      6,952,491 |      -2,272,688 (-24.64%) |
| Joint public-copy bytes   |  1,248,327,769 |    851,697,979 |    -396,629,790 (-31.77%) |

The integrated tip includes direct resident input advancement, direct arena
block operations, inline small definition regions, borrowed-node line
breaking, and semantic-lifetime command-context ownership since `.157`. The
single row cannot assign aggregate deltas among those changes. In particular,
the instruction, allocation, and copy reductions coexist with more cycles and
lower IPC in this probed row; no second run exists to turn that observation
into a latency or variance claim.

## Next architectural simplification

Make every non-source resident input level one compact token-row machine.
`advance_resident_command_into` remains the largest disjoint owner at 11.05%
self / 16.03% inclusive, more than three times the next application symbol,
while stored-token plus macro-argument delivery alone accounts for 49,439,769
of 49,903,532 frame steps. Replay, attempt, durable, macro-body, and
macro-argument rows currently repeat common top indexing, rollback first
touch, active-source extraction, position settlement, parameter interception,
and exhaustion handling around their lifetime-specific word read.

Put those common scalars in one resident token-row header and retain only a
tagged lifetime-specific storage coordinate behind it. The hot transition then
performs one storage read and one shared settlement; source tokenization stays
separate, and replay, attempt, durable-definition, macro-body, and argument
owners keep their existing reclamation lifetimes. This is a representation
simplification of the input subsystem, not a cache, extra command path, or
ownership merger. It directly targets the remaining dominant measured owner;
no implementation is made here.

## Evidence

Ignored issue-private evidence is under
`target/umber2-66p0.8.40.166/evidence/`. SHA-256 values for `perf.data`, raw
copy data, symbolized copy report, flat self report, flat inclusive report,
counter receipt, engine stderr, outer timing, key closure, and checked
interposer are respectively
`77690b327517994c9707889e70c7f3b23d11ee5fa268fd0191a576f3b0cd1bda`,
`8d700dc035f43439f99f425cd84c6e243b7cce7486064ffdba02c2a4cd413d84`,
`02ea8bbe9beb08ab830a78db4e104649cea9a192b1a55199f1e9d13ea8fee0d2`,
`8935139dde381c82a824273d739df24df7b54218f1f64b98e298576114962e75`,
`9c02458be47247acd8bcba14dc0ccec544b06588bf9515e34b358658f5002480`,
`6a043baf9354d4cdead1d5a95df31b1119677b26691e9d903edc524506674732`,
`3e77a732b896524cbb369be7abb393374b8cb64e0b725725c0cce8add1357e35`,
`077616f9ed2b98fb141e6fde2061a854a1fd0230b15a27fe6db6ffae9aa1854d`,
`e4f4113c9057af88c239d40d3041f598871a0a7a895f8bd63f89d7c77682ab7e`,
and
`8afd6ca34d91c28fccbccff4273a2f47dfa61861d3bceb53cf2c14dc7d2be16e`.
