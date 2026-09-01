# `umber2-66p0.8.40.118`: direct compact source projection

## Exact integrated authority

Exactly one execution entered the TeX engine on integrated commit
`e95158f78e49f6a96ef1c0af7e935085f081ed1b`. The optimized frame-pointer
binary SHA-256 was
`09a414b21bebe44cde1126fe875f521736c5291beca7b9aebacd949f1273d9b3`; the
checked public-copy interposer SHA-256 was
`8afd6ca34d91c28fccbccff4273a2f47dfa61861d3bceb53cf2c14dc7d2be16e`.
One pre-execution `perf` launch failed to allocate its requested 64 MiB ring,
and one later process start rejected a binary built without the required
`profiling` feature. Both stopped before source registration or command-fuel
consumption and are preserved separately from the authority row. The successful
capture used an 8 MiB ring, ordinary concurrent host conditions, and no CPU
hold or serialization.

The finite workload remained arXiv `2606.12566`: source SHA-256
`816440f61d611fa57cef802e6f372b9337beef1cc4e48e5536d4bad1014ec537`,
schema-12 format SHA-256
`ddd082db722654fd9c47e1080d8e128d1a5b0cfb3186f9e3bc01f8943f559ad4`,
offline distribution manifest SHA-256
`a68acebc1a83fd4ec0ce8c3baed4e8fe01de9b37e5a878b2f1fc203c2f20662f`
with aHash64 `df66c327ae636145`, ordered 123-key closure SHA-256
`e4f4113c9057af88c239d40d3041f598871a0a7a895f8bd63f89d7c77682ab7e`,
and source date epoch `1787080434`. The guards were exactly 20,000,000
canonical-command fuel and 40,000,000 committed executor steps. Expected status
1 occurred at the exact work vector `(20000000, 19915373, 2218784, 6021645,
16786906, 4011)` for fuel charges, token-frame steps, expanded deliveries,
meaning lookups, scanner tokens, and write expansions. Raw deliveries were
source `463231`, stored/body `11526849`, macro argument `7925202`, and synthetic
end-v `91`.

The combined wrapper reported 9.11 s wall, 6.28 s user, 0.64 s system, and
224,924 KiB peak RSS. The 199 Hz `cycles:u` capture spans 7.718 s, contains
1,292 samples, reports zero lost samples, and sums to approximately
14,859,419,561 weighted cycles.

## CPU attribution

The ranked self table below excludes the interposer's `record_copy` leaf
(5.84%) and public `memcpy` wrapper (0.63%), which are separately visible probe
overhead. The shared libc copy kernel is 1.04% self; because this libc resolves
public `memcpy` and `memmove` to the same address, those cycles cannot be split
honestly between the two APIs.

| Rank | Application self owner                    |   Self |
| ---: | ----------------------------------------- | -----: |
|    1 | `advance_resident_command_into`           | 14.39% |
|    2 | `raw_delivery_entry`                      |  4.44% |
|    3 | `expand_into`                             |  3.21% |
|    4 | `ArenaListView::cursor_span_at_node`      |  2.03% |
|    5 | `TracedTokenWord::pack`                   |  1.53% |
|    6 | `scan_toks_buffers`                       |  1.47% |
|    7 | `ExecutionScratch::append_argument_token` |  1.45% |
|    8 | `ForkArena::clone_chunk_prefix_from`      |  1.31% |
|    9 | `ForkArena::slice_direct_root`            |  1.22% |
|   10 | `Option::get_or_insert_with`              |  1.15% |

Inclusive rows overlap by definition. Their leading application owners were
`expand_into` 35.07%, `expanded_delivery_loop` 30.35%,
`execute_direct_episode` 30.24%, `raw_delivery_entry` 22.90%,
`advance_resident_command_into` 21.48%, `Universe::with_command_context`
20.14%, `preflight_command_into` 18.00%, `scan_toks_buffers` 17.13%,
`scan_command` 9.62%, and cold apply 9.59%. Probe `memcpy` ancestry was 6.71%
inclusive and `record_copy` remained 5.84% self/inclusive, so neither is
presented as application work.

## Exact copy reconciliation and selection

| API       |     Calls |         Bytes | Overflow calls / bytes | Probe-internal calls |
| --------- | --------: | ------------: | ---------------------: | -------------------: |
| `memcpy`  | 7,004,949 | 1,050,690,257 |                  0 / 0 |                    0 |
| `memmove` |   126,226 |    21,862,414 |                  0 / 0 |                    0 |
| Joint     | 7,131,175 | 1,072,552,671 |                  0 / 0 |                    0 |

Every API total equals its caller call/byte sum. `memcpy` recorded 72,364
collision probes with maximum displacement two; `memmove` recorded none. The
leading row, `ChunkStorage<Node>::release_lineage`, is 2,327,391 calls and
391,001,688 bytes and belongs to `.113`, so this issue excludes it. The leading
`memmove` row, `PageMaterialArena::push_active_list`, is 113,070 calls and
18,995,760 bytes and likewise overlaps node storage. The next independent
ordinary source path is
`SourceCursor::next_compact_unicode_step`: 367,847 copies of exactly 168 bytes,
61,798,296 bytes total. It materialized the owned-name-width
`ScannedSourceToken` for a character and immediately destructured that carrier
to borrow the character token. TeX semantics require the character, catcode,
range, and current meaning lookup, but not that intermediate transport value.

## Architecture and focused before/after gate

The shared tokenizer now accepts distinct owning and compact projections for
ordinary tokens. Ordinary characters write directly through the selected
projection. Escape scanning alone returns `ScannedControlSequence`, whose
borrowed-word variant still consumes the current source-line slice immediately
and whose owned variant still covers reduced, exact-byte, symbol, active,
paragraph, and null names. Public tokenizer consumers retain their owned
`SourceToken`; command delivery retains the same packed word and direct
provenance. No cache, alternate lexer, preprocessing, unsafe code, or semantic
state was added.

The focused `warmed_keyword_mismatch` row performs 16,384 complete warmed
scanner decisions over 147,465 direct source characters. The same checked
interposer and one `cycles:u,instructions:u` `perf stat` execution were used on
each exact binary. Both rows report the same PASS result, zero warmed
allocations/requested bytes, and four zero-byte `memmove` calls with zero table
overflow.

| Counter                                   |             Baseline |       After |                  Delta |
| ----------------------------------------- | -------------------: | ----------: | ---------------------: |
| Internal elapsed nanoseconds per decision |             3,981.20 |    3,403.71 |      -577.49 (-14.51%) |
| User cycles                               |          144,026,258 | 119,320,391 |  -24,705,867 (-17.15%) |
| User instructions                         |          334,947,940 | 303,231,253 |   -31,716,687 (-9.47%) |
| Public `memcpy` calls                     |              180,359 |      32,890 |               -147,469 |
| Public `memcpy` bytes                     |           27,926,279 |   3,151,484 |            -24,774,795 |
| Selected 168-byte tokenizer caller        | 147,465 / 24,774,120 |       0 / 0 | -147,465 / -24,774,120 |
| Public `memmove` calls / bytes            |                4 / 0 |       4 / 0 |                  0 / 0 |
| Warmed allocations / requested bytes      |                0 / 0 |       0 / 0 |                  0 / 0 |

Thus the selected carrier disappears exactly, whole-process copies fall by
nearly the same volume, and the focused CPU result improves without changing
semantic work or allocation.

## Validation and evidence

`cargo test -q --tests -p tex-command` passes 383 unit and 23 boundary tests.
The complete `cargo test -q --tests` run passes every suite except
`pdf_parity::committed_embedded_font_fixtures_match_bytes_structure_and_attestations`;
that deterministic embedded-Type1 fixture mismatch reproduces outside this
change and is already tracked by `umber2-emmj`. The complete routine suite with
only that exact known test skipped passes. `scripts/check.sh` reports all four
repository gates passed: dprint, Biome, rustfmt, and both clippy resolutions.

Ignored evidence is under `target/umber2-66p0.8.40.118/`. The authority receipt,
`perf.data`, raw copy report, symbolized copy report, self report, inclusive
report, and timing receipt SHA-256 values are respectively
`9f79ecf67491c8e9e9206ec3fac9495866c39306d2e35370c272ccf55086d614`,
`1dd91318c1cc068a2b40572a1011607416336730c296ebf2107be021373eea6e`,
`fad2ff937c9031e9f245b3f1b6f0cc4122763a40ae3df389cc0cf9283c921dce`,
`0d707a2220c9b5e865c8595378621724d2a8d3649c0f797d55b94cf86a605355`,
`982565b793f9ece1200d89840b48f674acb4872c50aa776ff7652abcc12ad909`,
`7e9418149412179621c308f2ec4ac086feccc94bdad4e257be1f4c4eb6adf7dc`,
and `d3fb394e1b318ab71cd16b120f38a10657aedcf1215f736623c2d861e43fd96d`.
The focused baseline and after symbolized reports have SHA-256 values
`dbbf0d1932a2b93e62d75748d111c280022bea93b2a9705cb1f7c7a5a5a51b43`
and `1d4fbc2cb2ba74dad9b5651920b97395259204093716ab5fd3f2f6f2de2e9069`;
their counter receipts are
`544ba9ab21af856e55b9870e520b55c0e22a4334ea84ec99dd5e665b55ae6de9`
and `9186e394b0205818dd409de998295afcf78c9418680304e004bf2f22821b57a9`.
