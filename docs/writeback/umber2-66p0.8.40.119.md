# `umber2-66p0.8.40.119`: remove the resident-domain profiling census

## Selection from the exact 20M authority

Issue `.118`'s one authenticated integrated arXiv `2606.12566` capture is the
sole broad-profile authority. Its application self table, after excluding the
copy probe, ranks
`CommandState::advance_resident_command_into` first at 14.39% self and 21.48%
inclusive. The higher public-copy owners are ForkedArena/node storage assigned
to `.113`; the active DVI paragraph frontier is likewise excluded. No new broad
profile was run.

The resident transition must select the semantic top-row variant, load and
advance its cursor, resolve the packed word, apply scanner/alignment semantics,
and charge canonical work. The `profiling` resolution additionally incremented
one saturating counter for that already-selected row domain. This second census
was not TeX state, did not drive control flow, and repeated the required enum
classification once per token only so focused fixtures could recover domain
volumes they had admitted themselves. The exact profile's annotated code
contains samples on those counter updates, including the macro-body, replay,
and macro-argument branches.

## Architecture change

`InputCursorMutationCounters` and its source, replay, durable, attempt,
macro-body, and macro-argument updates now exist only under `cfg(test)`, where
structural tests still assert exact dispatch. Ordinary and profiling builds
perform the mandatory top-row variant dispatch but retain no parallel resident
domain census. The focused benchmark publishes its known admitted replay,
attempt, and durable word volumes directly, while continuing to measure the
canonical fuel, token-frame, meaning-lookup, and expanded-delivery counters.

This deletes a counter owner and six updates. It adds no cache, fast path,
threshold, unsafe code, allocation, or replacement classifier. The optimized
focused symbol shrinks from 12,957 to 11,839 bytes, a reduction of 1,118 bytes
(8.629%).

## Focused before/after gate

The existing `fused_raw_expanded_delivery` row admits exact replay,
attempt-local, and durable spans, then performs 1,000,000 raw and 1,000,000
expanded stored-control-sequence deliveries through the production transition.
Both rows pass with zero allocation calls/requested bytes, zero intermediate
relays, zero whole-command/input copies, 2,000,000 fuel charges, 2,000,000
token-frame steps, 2,000,000 meaning lookups, and 1,000,000 expanded
deliveries. The candidate also reports the fixture's exact 666,667 replay,
666,666 attempt, and 666,667 durable words without counting those domains in
the hot loop.

One `cycles:u,instructions:u` execution and the checked `.118` public-copy
interposer were used for each exact binary. The first baseline launch
incorrectly preloaded the interposer into `perf` itself and its mixed caller
report failed reconciliation; it is excluded. The valid launch scopes the
interposer to the benchmark child, and both valid reports reconcile without
overflow or probe-internal calls.

| Counter                             |         Baseline |            After |                 Delta |
| ----------------------------------- | ---------------: | ---------------: | --------------------: |
| User cycles                         |      509,073,884 |      476,247,801 | -32,826,083 (-6.448%) |
| User instructions                   |    1,229,651,590 |    1,220,985,323 |  -8,666,267 (-0.705%) |
| Warmed allocations/requested bytes  |            0 / 0 |            0 / 0 |                 0 / 0 |
| Public `memcpy` calls/bytes         | 124 / 24,338,434 | 132 / 24,338,335 |              +8 / -99 |
| Public `memmove` calls/bytes        |            2 / 0 |            2 / 0 |                 0 / 0 |
| Hot resident-delivery public copies |            0 / 0 |            0 / 0 |                 0 / 0 |

The whole-process `memcpy` difference is eight startup calls and 99 fewer
bytes; neither report attributes a public copy to resident delivery. Thus the
removed 2,000,000 counter updates do not move their work to allocation,
`memcpy`, or `memmove`. The benchmark's short internal wall timer was slower in
the candidate under ordinary host frequency variation, so it is not presented
as an improvement; the material result is the hardware-work reduction from the
single paired counter executions.

The independent `mixed_macro_resident_pipeline` row also passes with its known
2,000,000 macro-body transitions, 1,000,004 replay words, 2,000,004 raw frame
steps, 1,000,000 expanded deliveries, 1,000,001 macro expansions, zero
suspension moves, zero command copies, and zero warmed allocation.

## Validation and evidence

`cargo test -q --tests -p tex-command` passes 383 unit and 23 boundary tests.
The complete `cargo test -q --tests` run passes every suite except
`pdf_parity::committed_embedded_font_fixtures_match_bytes_structure_and_attestations`;
that deterministic embedded-Type1 fixture mismatch reproduces from `.118`, is
unrelated to this command-only change, and remains tracked by `umber2-emmj`.
The complete routine suite with only that exact test skipped passes.
`scripts/check.sh` reports all four repository gates passed: dprint, Biome,
rustfmt, and both clippy resolutions.

Ignored evidence is under `target/umber2-66p0.8.40.119/`. Baseline binary,
symbolized copy report, and hardware-counter receipt SHA-256 values are
`37ff99949a3a29844ec5be9feb2c3d257fbbb80bd1e1dc03a84864e6709b8ede`,
`93635e88d3cec00ac3a12a78eaaf7db33ecde0f26029efe9c68631c57ecb04d2`,
and `de362af599ded0f38fe53af3b7086619dbd9b2d7cf8c90b806e40558d123e7d2`.
Candidate values are
`a92b573e5cda64c2e91c6acd3a1f8c95115f14daa035067f0bf5827c74acfead`,
`b992d176fb6a46bf685ab9ae4fc96f7d9f6f44525c38615bbdc8350d6ad62d06`,
and `b95417e08c70bd76d33830c462770e6b6d430f9f42295b219f3c4938f0cd9350`.
