# umber2-awgc.5.3.1: Exact Macro-Owner Index

## Authority

The final implementation is commit `76e6db23b`. The optimized profiling binary
has SHA-256
`51564fea5fb18a2d1eca494a860ddbbbc72421c7d05fe2aa21739235aa64ddd6`.
It reuses the immutable source, schema-11 format, schema-3 distribution,
ordered 105-key closure, offline cache, 120-second runtime timeout, and pinned
6M/12M command vectors from [`umber2-awgc.5.2`](umber2-awgc.5.2.md).

The baseline perf recording attributed 56.64% of exact 6M user cycles to
`PackedMacroChunkOwner::contains`. `ParameterState::admit_macro` linearly
searched every admitted immutable chunk whenever its single dense definition
slot named another generation. This was the dominant expansion cost, not the
expansion-command DTO or interpreter facade.

The replacement keeps the one persistent canonical interpreter. It adds a
dense 64-record chunk coordinate and an intrusive exact-generation chain per
recyclable definition slot. A hot definition now resolves to its first
immutable owner with dense and vector coordinates. Older generations remain
directly addressable for active replacement levels and detached snapshots.
There is no second executor and no hot hash table, `Arc`, `Weak`, allocation,
or scan across admitted owners. Macro activation also carries the admitted
owner through the existing interpreter borrow instead of resolving it twice
again.

## Fixed-boundary results

Both rows returned typed status 1 at the requested fuel boundary and preserved
the frozen work vectors exactly.

| Boundary |   Wall |   User | System |    Peak RSS | Expanded deliveries | Meaning lookups | Scanner tokens | Write expansions |
| -------- | -----: | -----: | -----: | ----------: | ------------------: | --------------: | -------------: | ---------------: |
| 6M       |  9.76s | 10.54s |  1.21s | 551,948 KiB |             507,410 |       1,718,333 |      5,352,087 |              588 |
| 12M      | 21.15s | 23.24s |  2.46s | 874,616 KiB |           1,177,349 |       3,506,292 |     10,599,869 |            1,182 |

The frozen 12M baseline was 47.19 seconds wall, 52.93 seconds user, and 874,604
KiB peak RSS. The exact-generation index therefore gives a 2.23× wall and
2.28× user speedup with a 12-KiB RSS difference. The frozen 6M row was 18.69
seconds wall; a same-session diagnostic repeat was 19.63 seconds, so the final
row is 1.92× to 2.01× faster depending on the baseline run.

The 6M stderr and time receipts have SHA-256
`cff4db50d9020da13a999e055fd9b2e9e2144f53f99eaae5bc5bc5a084694218`
and
`1300f6e2339fc2816b28ad97a10f8390c2c920e72ce0203a0860f4bc2f52d55d`.
The corresponding 12M hashes are
`e85cc30f43240544291ff83a0cd8c1edb7cf3d93f2b205289b27330498cdc2a2`
and
`e909cc3196bba670abe964013ce3a47407296589c672cd84f4ca6872538c9c1e`.
Local raw evidence is under
`target/umber2-awgc.5.3.1/generation-chain-{6m,12m}`.

## Allocation and semantic controls

The warmed packed cutover gate reports zero allocations, requested bytes,
`Arc` retains, `Weak` retains or upgrades, weak-index calls, and content-hash
calls for ordinary source delivery, packed backup/replay, stored-token replay,
and macro matching/replay/expansion. The census sees only four additional
amortized vector-growth calls relative to the frozen rows. At 12M, peak RSS is
effectively unchanged; there is no per-token allocation introduced by the
index.

The exact command-stream tracer compared every registered fixture through
exhaustion, including Plain, Story, and Gentle, and reported zero ordered
divergences and zero advisory geometry differences. Focused tex-command tests
cover direct packed-chunk indexing, exact-generation reuse, replacement-owner
coordinate survival, activation ownership, provenance, suspension, and replay.
`cargo test -q --tests` passes. `scripts/check.sh` reports all four gates
passed.

The final perf recording still assigns 21.72% of samples to the small
`PackedMacroChunkOwner::contains` leaf used by cold owner construction and
fallback. The eliminated all-owner search is not reintroduced: exact admitted
generations use the slot-local chain. Further work should measure owner
construction and packed-chunk copy-on-write directly rather than infer it from
this folded leaf symbol.
