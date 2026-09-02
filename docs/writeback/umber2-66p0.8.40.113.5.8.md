# `umber2-66p0.8.40.113.5.8`: live node-owner retirement

## Decision

Accepted. The live peak was not vacant backing. The profiling owner census
showed that the native one-shot frontend retained edit-restart checkpoints
through the same 64 MiB number used for its resource cache. At page output a
2,621-block historical page region overlapped a 2,653-block current region and
the bounded output/closure transition. Those roots could never serve a future
edit because the native run consumes the session after finalization or failure.

Restart-history retention is now an independent session policy. Persistent
editor and Wasm sessions keep the configured checkpoint budget; the native
one-shot frontend selects zero while retaining the unchanged resource cache.
Removing the final checkpoint row from a rootless current region also returns
its logical node and annex envelopes immediately instead of waiting for the
next named boundary. `PageRegionHistory` charges its shared `NodePool` backing
once, excluding warm vacant superblocks. Exact backing stays warm on vacancy,
so the fix does not repeat 64 KiB allocations at every page/box transition.

## Authenticated 50-million-command row

The final pinned row used profiling binary SHA-256
`7ed458e7e8d0b9bebdd0a0a46bb0f8073c13544c74501909662f966cc499a8ff`,
the established checked copy probe, arXiv `2606.12566` `ArXiv.tex`, schema-12
format, ordered 123-key closure, source epoch `1787080434`, and distribution
aHash64 `df66c327ae636145`. Guards remained 50,000,000 canonical-command fuel,
100,000,000 executor steps, 90 seconds, and 1,536 MiB aggregate RSS. Expected
status 1 preserved the exact semantic vector
`(50000000,49903532,9457781,15936698,35326903,4203)`.

Peak node backing fell from 8,185 blocks / 536,412,160 bytes to 5,198 /
340,656,128 (36.49%); annex backing fell from 132 / 8,650,752 to 80 /
5,242,880 (39.39%). RSS fell from 651,640 to 495,716 KiB (23.93%), materially
toward the historical 285,464-KiB row. At the largest sampled semantic owner
boundary the one-shot session had zero retained page regions and zero
checkpoint rows: 2,653 node blocks were current page state and 96 were
durable/output/other transition state. The remaining storage-event high-water
is the bounded page/output construction overlap, not stale restart history.

Named allocation traffic is 3,359,974 calls / 26,861,467,230 requested bytes,
below the warm predecessor's approximately 27.1 GiB and far below the stacked
release policy's 59.8 GiB. Fresh exact node allocations fell from 8,185 to
5,198 while 512,483 vacant-block reuses remained allocation-free. Joint public
copy is 9,589,922 calls / 1,472,615,235 bytes, also below the stacked row.

## Correctness and evidence

The page-region tests cover immediate final-row retirement, warm reuse,
stale-key rejection, checkpoint rollback, candidate settlement, and page
succession. Full profiling `tex-state`, `tex-incr`, targeted DVI/PDF parity,
and native CLI tests pass. The Wasm retained-editor workload remains stable
and improves from 623,771,648 to 96,927,744 bytes of linear-memory growth,
showing that persistent checkpoint semantics remain bounded and exact.

Ignored final evidence is under
`target/umber2-66p0.8.40.113.5.8/evidence-final/`. Raw copy, complete
symbolization, timing, and stderr SHA-256 values are respectively
`2aac08b327dcd75eb1b863379d500fcbcc7b7146fa9b0ed59b13aed5aa838a5a`,
`7626af0b441a924ecdebd01f007e5f47e446e5bd6bc9b67952143fa2394a6ebd`,
`025ad95b4300ad9856d043e3ebe875808b75be6b920fbf09e62358246255076e`,
and `9ac701e7d2251136a6616cd9d54fc2c70f7dfb533aee0638a994e9b6a86c204a`.
