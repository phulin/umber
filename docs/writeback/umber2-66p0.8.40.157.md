# `umber2-66p0.8.40.157`: packed-node and durable-box profile

## Authenticated bounded capture

Exactly one current-tree arXiv execution entered the engine at commit
`1ec291019c073d89596b9f864dc0ebcb8291b796` (tree
`0598ceff0a6eed699a3329b50eeb1af3cc12fa9b`). The Rust 1.93.0 profiling
binary has SHA-256
`84732cf4b6a608d368122a1f40670abb19d1ef3a31d6dbc36e305913e761b614`,
ELF build ID `1df14a20cafe67c27e9134312f519468103fc9d3`, and size 424,097,600
bytes. The checked public-copy interposer has SHA-256
`3378f994509f85dac45d1f2c1c41453f3f447facf91a5319e3d2d15f2410b686`.
The capture used 199 Hz `cycles:u`, 8,192-byte DWARF callchains, and an 8
MiB ring. It ran with normal concurrent host activity and no CPU hold or
serialization; no second authority execution or measurement series ran.

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

The capture contains exactly 3,371 samples and reports zero lost samples. Its
sampled periods sum to approximately 40,012,322,939 cycles. The simultaneous
counter row records 40,323,285,089 user cycles and 71,563,607,021 user
instructions (1.77 instructions/cycle) over 19.098000477 seconds. The outer
guard observed 19.32 seconds wall, 19.54 user, 2.19 system, and 166,820 KiB
peak RSS. These remain attributed, probed measurements rather than an
uninstrumented latency claim.

## CPU and storage attribution

Self percentages are disjoint sampled owners. Inclusive percentages overlap
through ancestry and must not be added.

| Owner                                     | Inclusive |   Self |
| ----------------------------------------- | --------: | -----: |
| `advance_resident_command_into`           |    17.23% | 11.68% |
| `expand_classified_into`                  |    31.23% |  3.17% |
| `ForkArena::admitted_chunk_value`         |     3.05% |  3.05% |
| `ForkArena::append_unsealed_list`         |     3.01% |  2.80% |
| `raw_delivery_entry`                      |    14.76% |  2.68% |
| `ArenaListView::get_cursor`               |     2.03% |  2.03% |
| `expanded_delivery_entry`                 |    28.65% |  1.74% |
| `append_reencoded_chunk_range`            |     5.29% |  1.63% |
| `ExecutionScratch::append_argument_token` |     2.04% |  1.61% |
| leading `Universe::with_command_context`  |    21.91% |  1.13% |

The copy probe itself accounts for 3.21% self in `record_copy` and 0.43% in
its `memcpy` wrapper. Two additional `with_command_context` monomorphs account
for 0.49% and 0.48% self, so the directly observed facade family is at least
2.10% disjoint self.

The exact named allocation census is:

| Allocation owner           |         Calls |    Requested bytes |
| -------------------------- | ------------: | -----------------: |
| `delivery_and_scan`        |       414,672 |      8,885,226,036 |
| `semantic_apply`           |     2,747,936 |        580,846,650 |
| `evidence_publication`     |         3,670 |          1,569,635 |
| `cold_materialization`     |       179,074 |     17,040,745,940 |
| `attempt_scratch`          |           665 |          1,668,720 |
| all remaining named owners |             0 |                  0 |
| **Exact total**            | **3,346,017** | **26,510,056,981** |

The node pool reached exactly 68 live node blocks / 4,456,448 payload bytes
and 80 live annex blocks / 5,242,880 payload bytes. It ended with zero live
blocks in both lanes. Node fresh/reuse/release totals were `68/1560/1622`;
annex totals were `80/36962/37031`. The largest sampled owner state held 22
current-page node blocks plus four durable/other blocks and 66 current-page
annex blocks plus nine durable/other blocks. Across 873 page installations,
872 were zero-copy takes and one was an on-demand promotion. The `.153`
instrumentation predates this pool census, so no fabricated `.153` block peak
is presented; the committed `.113.5.9` causal row independently records the
packing change from 5,198 node blocks to the same 68-block peak.

## Exact public-copy result

The tables reconcile every caller with zero overflow or probe-internal calls:

| API       |     Current calls / bytes |      `.153` calls / bytes |     Delta calls / bytes |
| --------- | ------------------------: | ------------------------: | ----------------------: |
| `memcpy`  | 9,203,569 / 1,245,500,251 | 9,584,526 / 1,476,059,455 | -380,957 / -230,559,204 |
| `memmove` |        21,610 / 2,827,518 |        13,974 / 2,666,350 |       +7,636 / +161,168 |
| Joint     | 9,225,179 / 1,248,327,769 | 9,598,500 / 1,478,725,805 | -373,321 / -230,398,036 |

The leading exact `memcpy` caller bins are:

| Symbolized caller                                               |   Calls |      Bytes |
| --------------------------------------------------------------- | ------: | ---------: |
| `Vec::clone` allocation/copy (`0x15ca8df`)                      | 373,119 | 71,638,848 |
| `NodeCursor::try_for_each_range` option projection (`0xdd5ddb`) | 122,706 | 20,491,902 |
| `NodeCursor::try_for_each_range` nested closure (`0xdd5e14`)    | 122,706 | 20,491,902 |
| second range monomorph option projection (`0x135a7e9`)          | 122,706 | 20,491,902 |
| second range monomorph callback (`0x135a816`)                   | 122,706 | 20,491,902 |

The leading exact `memmove` caller bins are:

| Symbolized caller                    | Calls |     Bytes |
| ------------------------------------ | ----: | --------: |
| `CandidateRun::run` (`0xeeb4a5`)     |   128 | 1,493,888 |
| `BTree` leaf insertion (`0x12dafa0`) | 3,158 |   313,704 |
| `BTree` leaf insertion (`0x156c295`) |   679 |    95,456 |
| `BTree` leaf insertion (`0x156c2f2`) |   679 |    71,592 |
| `BTree` leaf insertion (`0x15752c0`) | 3,284 |    57,736 |

The complete symbolized report retains the top 40 exact caller bins for each
API and the raw report retains every bin.

## Comparison with `.153`

The semantic vector and all four raw-delivery subtotals are byte-for-byte
equal to `.153`.

| Measure                   |          `.153` |        Current |                    Change |
| ------------------------- | --------------: | -------------: | ------------------------: |
| Outer wall                |         49.36 s |        19.32 s |        -30.04 s (-60.86%) |
| Outer user                |         50.23 s |        19.54 s |        -30.69 s (-61.10%) |
| Outer system              |          5.32 s |         2.19 s |         -3.13 s (-58.83%) |
| Simultaneous user cycles  | 105,335,209,814 | 40,323,285,089 | -65,011,924,725 (-61.72%) |
| Simultaneous instructions |  86,497,658,833 | 71,563,607,021 | -14,934,051,812 (-17.27%) |
| Peak RSS                  |     692,396 KiB |    166,820 KiB |    -525,576 KiB (-75.91%) |
| Named allocation calls    |       3,363,123 |      3,346,017 |          -17,106 (-0.51%) |
| Named requested bytes     |  27,068,520,414 | 26,510,056,981 |     -558,463,433 (-2.06%) |
| Joint public-copy calls   |       9,598,500 |      9,225,179 |         -373,321 (-3.89%) |
| Joint public-copy bytes   |   1,478,725,805 |  1,248,327,769 |    -230,398,036 (-15.58%) |

The current profile includes the packed node chunks, one-shot restart-root
retirement, resident token-row header, occupied command destination, and
in-place durable-box ownership integrated since `.153`. The single combined
row cannot assign its aggregate deltas among those changes. The separately
committed focused and authority receipts remain the causal evidence for each.

## Next architectural simplification

Make command-visible state one directly borrowable resident owner. The first
two ranked non-node `.153` targets are now implemented by `.154` and `.155`;
the third remains in current source. `Universe::with_command_context` still
reassembles the broad reference facade for each processor/application episode,
with at least 2.10% disjoint self across three visible monomorphs and 21.91%
inclusive ancestry at the leading entry. Group those fields under one Universe
subobject and lend that owner directly through the existing episode, deleting
repeated facade construction and field-by-field lookup. Existing borrow
lifetimes and tracked admission remain authoritative; this needs no cache,
persistent alias, alternate command path, or new semantic state. No
implementation is made in this issue.

## Evidence

Ignored issue-private evidence is under
`target/umber2-66p0.8.40.157/evidence/`. SHA-256 values for `perf.data`, raw
copy data, symbolized copy report, self report, inclusive report, counter
receipt, engine stderr, outer timing, key closure, and checked interposer are
respectively
`54f3e5ad14637996db9fd9a32a77b27ebf5c64b61fbbb11b22eb2a60cd15404a`,
`cedea2609afe148762c57c159d1c1ec3a701a064a5e719c78bec7fe5bcf502ad`,
`0b2d0aed2b4aa4fd2649cc3620bfce47f22af837119614e7fb03b123e1c18ac5`,
`0366379c26b3cd59cc7c76ed1040c1f5add85046fdb5cac1adeb3ae81631bc43`,
`3487f1308e8cfab4056b86885fd5333e6dd1895940691150c611bbddb1109a0a`,
`8c80062281a8e4f71a0818219c45aa20474ee4ae6a43b05b02d1a637355e6905`,
`3d52b09b4a3052bf71a0bfd2c16cba5afaabcf436bc86daedac47117fd1a4368`,
`1ebd9547d49c84612b09f7faf40442e41980891c5df284dcc54d6c3b542b598a`,
`e4f4113c9057af88c239d40d3041f598871a0a7a895f8bd63f89d7c77682ab7e`,
and
`3378f994509f85dac45d1f2c1c41453f3f447facf91a5319e3d2d15f2410b686`.
