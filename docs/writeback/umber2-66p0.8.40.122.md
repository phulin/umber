# `umber2-66p0.8.40.122`: counter-free hot-loop profile and source-owner release

## Exact integrated authority

Exactly one execution entered the TeX engine on clean integrated commit
`fee987caa0d25a27be19bc1f164bba1eb4275d67` (tree
`60a0c1a01c1eff5d986bf2b698831655d76e8fb6`). The optimized profiling binary
SHA-256 was
`1718ebdaeaa59cd7668834e57887703677fff74cea6f218e3f7c5f20ceb8971d`; the
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

The wrapper reported 8.11 s wall, 6.44 s user, 0.54 s system, and 225,136 KiB
peak RSS. The capture contains 1,325 samples, zero lost samples, and
14,833,621,319 weighted cycles.

## CPU attribution

The self table excludes the interposer's `record_copy` leaf at 4.92% and its
public `memcpy` wrapper at 0.42%; these are probe overhead. The shared libc copy
kernel is 1.64% self/1.84% inclusive and cannot be divided honestly by API
after libc resolves both public calls to that implementation.

| Rank | Observed application self owner              |   Self |
| ---: | -------------------------------------------- | -----: |
|    1 | `advance_resident_command_into`              | 11.75% |
|    2 | `expand_into`                                |  3.37% |
|    3 | `raw_delivery_entry`                         |  3.15% |
|    4 | `ArenaListView::cursor_span_at_node`         |  2.10% |
|    5 | `ExecutionScratch::append_argument_token`    |  1.45% |
|    6 | `expanded_delivery_loop`                     |  1.39% |
|    7 | `CommandFuel::record_raw_delivery`           |  1.26% |
|    8 | `ForkArena::payload_reservation_target`      |  1.24% |
|    9 | `Option::get_or_insert_with`                 |  1.05% |
|   10 | `Universe::with_command_context`             |  0.98% |
|   11 | `AttemptTokenListView::get`                  |  0.96% |
|   12 | `SourceCursor::next_compact_exact_byte_step` |  0.81% |

Inclusive rows overlap. The leading observed application rows are
`expand_into` 36.37%, `expanded_delivery_loop` 29.56%,
`execute_direct_episode` 28.43%, `Universe::with_command_context` 21.16%,
`raw_delivery_entry` 20.99%, `advance_resident_command_into` 20.23%,
`preflight_command_into` 18.68%, `scan_toks_buffers` 15.59%, cold apply 11.47%,
and `scan_command` 7.59%. Public `memcpy` ancestry is 5.85% inclusive and
`record_copy` remains 4.92% self/inclusive.

The resident transition's self leader is now the required semantic loop:
semantic top selection, cursor load/advance, meaning resolution, brace and
alignment treatment, parameter interception, and canonical work accounting.
The 2.10% cursor-span row and other leading ForkedArena rows belong to dense
storage; paragraph, node-copy, and hmode owners remain under active DVI parity.
This issue changes neither area.

## Exact copy reconciliation and selection

| API       |     Calls |         Bytes | Overflow calls / bytes | Probe-internal calls |
| --------- | --------: | ------------: | ---------------------: | -------------------: |
| `memcpy`  | 6,688,758 |   996,946,585 |                  0 / 0 |                    0 |
| `memmove` |   126,598 |    21,914,254 |                  0 / 0 |                    0 |
| Joint     | 6,815,356 | 1,018,860,839 |                  0 / 0 |                    0 |

Every API total equals its caller call/byte sum. `memcpy` recorded 17,125
collision probes with maximum displacement two; `memmove` recorded none.
`ChunkStorage<Node>::release_lineage` leads at 2,337,264 calls and 392,660,352
bytes but is excluded dense storage. The next larger independent rows are
required cold PDF/image, checkpoint, immutable-resource, and decompression
payload movement, or hmode/DVI work.

The first material redundant application owner is one source-line owner
transition split across three compiler-emitted rows:

| Row                                              |   Calls |      Bytes |
| ------------------------------------------------ | ------: | ---------: |
| `acquire_input_top_line_with_queries` closure    |  56,964 | 16,861,344 |
| `InputStack::mutate_source` returned carrier     |  56,922 | 16,848,912 |
| `SourceLevelExecutionState::cursor` construction |  56,964 | 10,937,088 |
| Total                                            | 170,850 | 44,647,344 |

`mutate_top_source` required every source mutation closure to construct and
return a full cold `SourceLevelExecutionState`, even when no checkpoint could
reach the displaced owner or the row had already retained its one inverse for
the interval. Those intermediate states were immediately dropped. This is
nonsemantic ownership transport, not physical-line, tokenizer, rollback, or
redo work.

## Architecture and focused before/after gate

Input history now selects its semantic cursor, everyeof, or backing transition
before borrowing the row. The first checkpoint-reachable owner change still
moves the exact cold state directly into its generation-checked inverse slab.
Later changes in that interval, and all changes to an interval-local row,
release the displaced line and replacement owners in place. The mutation
closure returns only its semantic result; an optional slab handle, never the
cold state, crosses the borrow boundary. This removes the universal carrier
API without a cache, threshold, special document path, unsafe code, second
input representation, or change to rollback ordering.

The focused gate performs 1,999,999 physical-line owner transitions after one
observable checkpoint over the same two-million-line source. Both exact
binaries report PASS, one reachable owner swap, zero full-frame history clones,
and successful rollback followed by the same next-line transition. One
`cycles:u,instructions:u` `perf stat` execution and the checked copy probe were
used per binary.

| Counter                        | Exact `fee987caa` baseline | Final change |                  Delta |
| ------------------------------ | -------------------------: | -----------: | ---------------------: |
| Internal elapsed nanoseconds   |                303,533,645 |   77,709,633 | -225,824,012 (-74.40%) |
| User cycles                    |                675,807,433 |  182,570,229 | -493,237,204 (-72.98%) |
| User instructions              |              1,373,924,708 |  635,913,371 | -738,011,337 (-53.72%) |
| Public `memcpy` calls          |                  4,000,110 |          110 |             -4,000,000 |
| Public `memcpy` bytes          |                980,291,557 |    4,291,452 |           -976,000,105 |
| Public `memmove` calls / bytes |                      4 / 0 |        4 / 0 |                  0 / 0 |
| Owner swaps / frame clones     |                      1 / 0 |        1 / 0 |                  0 / 0 |

The two baseline hot rows total 3,999,996 calls and 975,999,024 bytes; both are
exactly absent from the final report. The remaining 110 calls are setup, the
four-million-byte source `Vec`-to-`Arc` admission, and one retained first-touch
inverse/rollback, not the repeated owner transition.

## Validation and evidence

`cargo test -q --tests -p tex-command` passes 384 unit and 23 boundary tests;
`cargo check -q -p tex-command --features profiling` passes; and the explicit
copy-attribution tool gate passes its scalar, `Vec`, external-ancestor,
external-only, reconciliation, and exact-binary checks. The complete
`cargo test -q --tests` run reaches only the pre-existing deterministic
`pdf_parity::committed_embedded_font_fixtures_match_bytes_structure_and_attestations`
failure tracked by `umber2-emmj`; the complete routine suite with exactly that
test skipped passes. `scripts/check.sh` reports all four gates passed: dprint,
Biome, rustfmt, and both clippy resolutions.

Ignored evidence is under `target/umber2-66p0.8.40.122/`. Authority receipt,
`perf.data`, raw copy report, symbolized copy report, self report, inclusive
report, and timing receipt SHA-256 values are respectively
`e75f77a80736137c5b8d54ec17a511173935a2105ef5f5fedbff1fdd92da19fd`,
`90ebae53a6a05455f9f177a4567691cf467457a5eaed91944166a920df2cc8c0`,
`dceb5e0c23b6b9f811000fdbde1ce6ea51bbbf1fc4ddd44422c1be9d66810d5f`,
`0aa0ca6eea078235ec6ef3fb100cc2206a5338a91d24ad75571ae57b41ce7e20`,
`0b6f95b5d6a3783f59f5a9759806d92f728fbaedd4bbaa6cd8f66b7a3f42046a`,
`080266bc1589d337751cb154de16d6420e349ccb05bb95e88626ec29f926d1bd`,
and `5e3b0d5c5a43b23f16fed03e5148de31fd94f89fa21087ef7d87365ab0e2fab4`.
Focused baseline/final copy-report SHA-256 values are
`0f9712b4fdbbe5e5bb70f26798e3b2cd4ad562bdd979906bf22c110bcf17b5a9`
and `b1d93dccd784d60c0caa8fd516c6bf43f465ce47fdbf4e7d001aa96ce720a610`;
their counter receipts are
`e8c233b101417a2cf98ad03b42ed17296f9c422168222b344c9a3084fedac93a`
and `d97fbd84b69fae8c2de491bf1b7f4b95c7ac1dc6deaaea01d58ed9c36d478555`.
