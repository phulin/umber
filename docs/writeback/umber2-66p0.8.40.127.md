# `umber2-66p0.8.40.127`: derive command-write volume from raw delivery

## Exact current-main authority

Exactly one authenticated execution entered the engine on commit
`bdb8ba4e8d08a6c43e375853712ade94fd513645`. The optimized profiling binary
SHA-256 was
`3ef12cfbdfc2d38beefbd6de6f362d560363ddb3ce9d2123fe7c30fd9f7ea613`;
the checked public-copy interposer was
`8afd6ca34d91c28fccbccff4273a2f47dfa61861d3bceb53cf2c14dc7d2be16e`.
The capture combined 199 Hz `cycles:u` DWARF callchains with exact public
`memcpy` and `memmove` attribution, an 8 MiB ring, and no CPU hold, affinity,
serialization, cache purge, control workload, fuel ladder, or second arXiv
execution.

The finite workload remained arXiv `2606.12566`: `ArXiv.tex` SHA-256
`816440f61d611fa57cef802e6f372b9337beef1cc4e48e5536d4bad1014ec537`,
schema-12 format SHA-256
`ddd082db722654fd9c47e1080d8e128d1a5b0cfb3186f9e3bc01f8943f559ad4`,
offline distribution manifest SHA-256
`a68acebc1a83fd4ec0ce8c3baed4e8fe01de9b37e5a878b2f1fc203c2f20662f`
with aHash64 `df66c327ae636145`, ordered 123-key closure SHA-256
`e4f4113c9057af88c239d40d3041f598871a0a7a895f8bd63f89d7c77682ab7e`,
and source date epoch `1787080434`. Guards were 20,000,000 canonical command
fuel and 40,000,000 committed executor steps. Expected status 1 occurred at
vector `(20000000, 19907047, 2216876, 6018541, 16781945, 4011)` for fuel
charges, token-frame steps, expanded deliveries, meaning lookups, scanner
tokens, and write expansions. Raw deliveries were source `463197`, stored/body
`11520843`, macro argument `7922916`, and synthetic end-v `91`.

The wrapper reported 11.94 s wall, 9.75 s user, 0.71 s system, and 223,064 KiB
peak RSS under concurrent host load. The capture contains 2,026 samples, zero
lost samples, and 23,554,254,644 weighted cycles.

## Current hotspot and copy ranking

`advance_resident_command_into` led application self time at 12.14%. The next
application owners were `raw_delivery_entry` at 3.82%, `expand_into` at 2.95%,
`ExecutionScratch::append_argument_token` at 2.24%, and
`ArenaListView::cursor_span_at_node` at 1.34%. The public-copy probe itself was
4.31% self and is reported separately. Dense-arena owners, the node-token
cutover, page material, and DVI-format work were excluded from selection.

Source annotation of the leading resident function showed the profiling-only
thread-local command-ownership update in every successful resolved delivery.
The same raw transition already increments the singular command-work ledger's
token-frame and raw-kind counters. TeX82 §§24--25 require one resolved current
command and its coordinate per successful raw delivery; they do not require a
second operational census of those writes.

Exact public-copy attribution reconciles 6,513,838 `memcpy` calls for
951,575,234 bytes and 126,700 `memmove` calls for 21,931,622 bytes, or
6,640,538 calls and 973,506,856 bytes jointly, with zero collision overflow or
probe-internal calls. The largest `memcpy` row was excluded dense storage:
`ChunkStorage::release_lineage`, 2,333,768 calls and 392,073,024 bytes. The
largest `memmove` row was excluded page material:
`PageMaterialArena::push_active_list`, 113,548 calls and 19,076,064 bytes. No
copy row motivated the selected CPU simplification.

## Architectural simplification

Production profiling no longer updates `resolved_writes` and
`delivery_stamp_writes` in a thread-local ownership census. Their exact volume
is derivable from the authoritative raw-delivery count because one successful
raw delivery writes each field exactly once. Test builds retain the direct
counters for structural assertions. Backup copies and suspension moves remain
separately counted because the raw-delivery vector does not imply them.

This removes one parallel nonsemantic state transition from the largest
application owner. It adds no threshold, cache, alternate delivery route, or
special-case fast path. The caller-owned current command, provenance,
suspension, rollback, retirement, acceptance, and TeX semantics are unchanged.

## Focused before/after gate

The exact baseline was current main before this change. Both release binaries
ran the production `mixed_macro_resident_pipeline` once under `perf stat` and
the checked public-copy interposer. Both report 2,000,000 macro-body words,
1,000,000 parameter deliveries, 1,000,004 replay words, 2,000,004 raw frame
steps, 1,000,000 expanded deliveries, 1,000,001 macro expansions, zero command
copies or suspension moves, and zero warmed allocations or requested bytes.

| Counter                              |      Baseline |         Final |                 Delta |
| ------------------------------------ | ------------: | ------------: | --------------------: |
| User instructions                    | 2,490,754,972 | 2,452,753,956 |  -38,001,016 (-1.53%) |
| User cycles                          | 1,066,253,920 |   980,735,053 |  -85,518,867 (-8.02%) |
| Internal elapsed nanoseconds         |   431,899,968 |   345,308,531 | -86,591,437 (-20.05%) |
| Nanoseconds per macro-body word      |        215.95 |        172.65 |      -43.30 (-20.05%) |
| Warmed allocations / requested bytes |         0 / 0 |         0 / 0 |                 0 / 0 |
| Public `memcpy` calls / bytes        | 130 / 344,172 | 130 / 344,169 |                0 / -3 |
| Public `memmove` calls / bytes       |         2 / 0 |         2 / 0 |                 0 / 0 |

The exact instruction reduction is the primary CPU result and is about 19
instructions per raw delivery. Cycles and elapsed time also decrease, but the
host was not serialized, so they remain supporting diagnostics. Public copy
calls are unchanged; the three-byte startup difference is not attributed to
the command loop. Both symbolized reports reconcile with zero overflow,
collision loss, or probe-internal calls.

## Validation and evidence

`cargo test -q --tests -p tex-command` passes 384 unit and 23 boundary tests.
The focused production benchmark and its allocation/copy invariants pass. The
full `scripts/check.sh` run passed biome, rustfmt, and both clippy resolutions;
its only failure was dprint's expected canonical rewrite of the new table. The
named `scripts/check.sh dprint` rerun passed after applying that formatting.

Ignored evidence is under `target/umber2-66p0.8.40.127/`. Authority
`perf.data`, raw copy report, symbolized copy report, self ranking, inclusive
ranking, and timing receipt SHA-256 values are respectively
`26d6b1daedd18d30edc138e2773df0c1ad999318156b3896bb23f73f47dd752d`,
`8b3a953be72de8d66e2752794bd82a39c12405fc287f9b674cf5bc4b2a95b41d`,
`cfc85dd44484131f0d5538d1d03158a14730491a99ac4c5d29b4536f4cc6187b`,
`2b5cf7b98e17b40e79b6db1e2b3d6ec4a0723e06eb0d619d8c916721af6763a7`,
`772e2bc5898885fc03ede6fbd29d1df78cf7bcb4000a5c6d56fd08c829b6e327`,
and `517a59b3e5c98684613a397f57c0aa113619a5321633c317b1d2479a9bd3d677`.
Focused baseline/final counter receipt SHA-256 values are
`36d2f2855f92aa44e65b8e67a52a9fc53d15534b6ea806d1f8540e25c4a4d073`
and `1ce870bf8a60b5c3bb7284d38efe0dd62f256c660ff3220bcde94f607f73c20e`;
copy reports are
`b656043f7d62bc71b1c4391601d0c47086123b94f5290571aa8697b46fdd2590`
and `8bbeafa3fb07109bb262afa48c08e3b3cac139e2969b9195accd1ec9da1cb8cf`.
Exact baseline/final binary SHA-256 values are
`10a97fbaedf778f566a92392abac2a882ac1a914ca02dc1b247f0640ce166e2c`
and `c26b5b3a173807a83979c834fa59fc869431756d2c276a539dd11486bd7c463b`.
