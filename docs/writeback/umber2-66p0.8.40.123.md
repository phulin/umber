# `umber2-66p0.8.40.123`: remove semantic construction from empty command slots

## Exact integrated authority

Exactly one execution entered the TeX engine on clean current-main commit
`b1b61d514c517d1fa10c3885109547911cbec10b` (tree
`de1fd6ab674389e6c7c9d18cdd6892042fa8644e`). The optimized profiling binary
SHA-256 was
`a010da0e94e7063c7a6f605025e7e9ace67e311f4b5b1522ddb019898247a3f4`; the
checked public-copy interposer SHA-256 was
`8afd6ca34d91c28fccbccff4273a2f47dfa61861d3bceb53cf2c14dc7d2be16e`.
The capture combined 199 Hz `cycles:u` DWARF callchains with exact public
`memcpy` and `memmove` attribution. It used an 8 MiB ring under ordinary
concurrent host load, with no CPU hold, affinity, serialization, cache purge,
control workload, fuel ladder, second arXiv execution, or hard wall cap.

The finite workload remained arXiv `2606.12566`: `ArXiv.tex` SHA-256
`816440f61d611fa57cef802e6f372b9337beef1cc4e48e5536d4bad1014ec537`,
schema-12 format SHA-256
`ddd082db722654fd9c47e1080d8e128d1a5b0cfb3186f9e3bc01f8943f559ad4`,
offline distribution manifest SHA-256
`a68acebc1a83fd4ec0ce8c3baed4e8fe01de9b37e5a878b2f1fc203c2f20662f`
with aHash64 `df66c327ae636145`, ordered 123-key closure SHA-256
`e4f4113c9057af88c239d40d3041f598871a0a7a895f8bd63f89d7c77682ab7e`,
and source date epoch `1787080434`. Guards were exactly 20,000,000 canonical
command fuel and 40,000,000 committed executor steps. Expected status 1
occurred at vector `(20000000, 19907047, 2216876, 6018541, 16781945, 4011)`
for fuel charges, token-frame steps, expanded deliveries, meaning lookups,
scanner tokens, and write expansions. Raw deliveries were source `463197`,
stored/body `11520843`, macro argument `7922916`, and synthetic end-v `91`.

The wrapper reported 10.15 s wall, 7.47 s user, 0.63 s system, and 223,600 KiB
peak RSS. The capture contains 1,565 samples, zero lost samples, and
17,868,314,546 weighted cycles.

## CPU and copy selection

The self table excludes the interposer's `record_copy` leaf at 4.94% and its
public `memcpy` wrapper at 0.53%; these are probe overhead. The shared libc
copy kernel is 1.23% self and cannot be divided honestly by API after libc
resolves both public calls to that implementation.

| Rank | Observed application self owner           |   Self |
| ---: | ----------------------------------------- | -----: |
|    1 | `advance_resident_command_into`           | 12.23% |
|    2 | `raw_delivery_entry`                      |  4.29% |
|    3 | `expand_into`                             |  3.53% |
|    4 | `ArenaListView::cursor_span_at_node`      |  2.04% |
|    5 | `ExecutionScratch::append_argument_token` |  1.81% |
|    6 | `expanded_delivery_loop`                  |  1.60% |
|    7 | `TracedTokenWord::pack`                   |  1.32% |
|    8 | `CommandFuel::record_raw_delivery`        |  1.13% |
|    9 | `scan_toks_buffers`                       |  1.05% |
|   10 | `ContentIdentity::for_domain`             |  1.04% |
|   11 | `Universe::with_command_context`          |  1.03% |
|   12 | `Option::get_or_insert_with`              |  0.97% |

The leading resident, raw, expansion, and scanner owners contain required
semantic work. The cursor-span row and the other leading fork-arena rows
belong to dense-superblock emplacement; node copy, hmode, paragraph, page, and
shipout rows remain under active DVI parity. This issue changes neither lane.

The first individually isolated nonsemantic leaf is
`TracedTokenWord::pack`. It owns 20 samples and 235,814,897 weighted cycles.
Thirteen samples and 152,424,162 weighted cycles, 64.64% of that leaf, have
`raw_delivery_entry` as their immediate parent. Exact-binary disassembly shows
why: whenever the caller's `Option<CurrentCommand>` was empty, safe raw
delivery constructed a fake NUL/other `Token`, ran the complete checked
semantic packer, initialized the command spelling with it, and immediately
overwrote it through the resident destination write. The fake token never
entered observation, input, rollback, or execution semantics.

Exact public-copy attribution reconciles as follows:

| API       |     Calls |       Bytes | Overflow calls / bytes | Probe-internal calls |
| --------- | --------: | ----------: | ---------------------: | -------------------: |
| `memcpy`  | 6,517,723 | 952,259,353 |                  0 / 0 |                    0 |
| `memmove` |   126,598 |  21,914,254 |                  0 / 0 |                    0 |
| Joint     | 6,644,321 | 974,173,607 |                  0 / 0 |                    0 |

Every API total equals its caller call/byte sum. `memcpy` recorded 8,282
collision probes with maximum displacement one; `memmove` recorded 67 with
maximum displacement one. `ChunkStorage<Node>::release_lineage` leads at
2,337,264 calls and 392,660,352 bytes and is excluded dense storage. The
leading `memmove` row is `PageMaterialArena::push_active_list` at 113,448
calls and 19,059,264 bytes and is excluded DVI/page material. No copy row
motivates or is changed by the selected CPU removal.

## Architecture and focused gate

`TracedTokenWord` now exposes one opaque initialized-placeholder constant for
safe destination storage. `CurrentCommand::empty` installs that representation
directly instead of constructing a semantic `Token`. The resident write still
overwrites spelling, origin, delivery coordinate, source facts, resolved
meaning, control-sequence identity, and delivery flags before the command can
leave the raw loop. No raw-word constructor is exposed, no unsafe code or
second command representation is introduced, and the placeholder never enters
a checkpoint or rollback root.

The existing focused fused-delivery gate performs one million raw and one
million expanded stored-control-sequence deliveries across exact replay,
attempt-local, and durable spans. Both exact binaries report PASS with zero
warmed allocations/requested bytes, zero intermediate relays, zero
whole-command/input copies, 2,000,000 fuel charges, 2,000,000 token-frame
steps, 2,000,000 meaning lookups, and 1,000,000 expanded deliveries. One
`cycles:u,instructions:u` execution with the checked copy probe was used for
each accepted row.

| Counter                                       |         Baseline |            Final |                Delta |
| --------------------------------------------- | ---------------: | ---------------: | -------------------: |
| User cycles                                   |      473,110,114 |      466,161,327 |  -6,948,787 (-1.47%) |
| User instructions                             |    1,211,985,081 |    1,151,983,356 | -60,001,725 (-4.95%) |
| Raw internal nanoseconds per delivery         |            78.77 |            76.73 |       -2.04 (-2.59%) |
| Expanded internal nanoseconds per delivery    |            89.56 |            87.69 |       -1.87 (-2.09%) |
| Warmed allocations / requested bytes          |            0 / 0 |            0 / 0 |                0 / 0 |
| Public `memcpy` calls / bytes                 | 132 / 24,338,380 | 132 / 24,338,377 |               0 / -3 |
| Public `memmove` calls / bytes                |            2 / 0 |            2 / 0 |                0 / 0 |
| Fuel / frame steps / meaning lookups          |     2M / 2M / 2M |     2M / 2M / 2M |                0 / 0 |
| Expanded deliveries / relays / command copies |       1M / 0 / 0 |       1M / 0 / 0 |                0 / 0 |

The exact 60,001,725-instruction reduction is the primary focused CPU result.
The three-byte `memcpy` difference is startup layout noise; no hot copy call
appears or moves to `memmove`. One earlier setup invocation accidentally
preloaded the `perf` parent as well as the benchmark, so two processes wrote
one copy report. The symbolizer rejected its non-reconciling totals. That
procedural artifact is preserved with an `invalid-parent-preload` name and is
excluded from every table above.

## Validation and evidence

`cargo test -q --tests -p tex-state -p tex-command` passes tex-command's 384
unit and 23 boundary tests and tex-state's 549 unit, 12 boundary, and one
structural lifecycle tests. `cargo check -q -p tex-command --features
profiling` passes. The explicit copy-attribution tool gate passes its scalar,
`Vec`, external-ancestor, external-only, reconciliation, and exact-binary
checks.

Ignored evidence is under `target/umber2-66p0.8.40.123/`. Authority receipt,
`perf.data`, raw copy report, symbolized copy report, self report, inclusive
report, and timing receipt SHA-256 values are respectively
`7c4414f02b9aae005e13bfb4b9f0646097cb5272ba2673ee379d13ccc1bf632f`,
`5a47de459e8150f9f4f79a9da5e03f1c6bb20f7cc30b7fe902ca7f4aeeb63355`,
`219e33fdf0f4daa7e581aaf1d22135696fc589b185238358d08cc774518195b2`,
`91aadb2e0ab3ad317b9b01ca93d00952487d1eb38eb621de7cbb2ad50e8fa697`,
`8c5756bef16e4f98b01b1d2f8a841f0a0f299bfcabfcb9053d2e1c95769a9d8f`,
`d0b90906780b90f224208177ca6335d18958db06c4a95feefbdd7bf76d508f80`,
and `9f5f7f6689c70b13b692dd284e8bdc9db651bd552971e42fbc331501ee03747a`.
Focused baseline/final copy-report SHA-256 values are
`f8acfa743718302c525299121e61bb50ab6406fa091f0d983fe4ced6addf8b5a`
and `2e70dec530f9f27bb50cef9264529a81e98284e402ecdc2da82cbe28e4cdc496`;
their counter receipts are
`aa98ccd6caa74bceb802d7eeeed9b60863b03722124391754e8ac385a04f6230`
and `be85b0a24f0ea61d244abb6289f4c15e2e574758175e2def074fc061e96dfa20`.
