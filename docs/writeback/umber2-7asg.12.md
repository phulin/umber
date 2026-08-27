# `umber2-7asg.12`: borrowed executor capability contexts

## Evidence boundary

The comparison reuses the authenticated arXiv `2606.12566` workload from the
combined copy audit: packed distribution root `721e833071d92bba`, schema-12
format object `ahash64-v1-2b924b5bba05d8a0`, the exact 123-key closure, a
fixed source clock, and a 20,000,000-action fuel boundary. The baseline
frame-pointer profiling executable has SHA-256
`5b39fc8c1eb2c724ad94b0c0dd4d1aaca21dc20beb7888079441f9f3d5cf6f20`
and build ID `7427532695e82663182e292eb2209ce9bdf64aab`. The candidate has
SHA-256
`d0dd4f0c27c7770b0c88911908c1f7545ce8a3832cb256e77c5d44cc7742199f`
and build ID `b417c0417beed03bfdf1fe7d3b20da961be62f26`.

Every accepted measured row was serialized with
`flock /tmp/umber-perf-host.lock`. Issue-private binaries, interposer output,
symbolization, perf data, process receipts, and gate logs live under
`target/umber2-7asg.12/`. The first candidate perf row is retained there but
rejected because its process receipt records concurrent Cargo and rustc work;
`perf-quiet-20m` is the accepted zero-loss row.

## Transfer proof and structural change

`CommandContext` is the 208-byte admitted, borrow-scoped capability value.
The baseline census identifies four redundant transfer families:

| Baseline owner                    |         Calls |           Bytes | Why redundant                                                                                                    |
| --------------------------------- | ------------: | --------------: | ---------------------------------------------------------------------------------------------------------------- |
| `CommandProcessor::from_parts`    |     1,063,524 |     221,212,992 | The facade moved the already-admitted context twice while constructing an episode that only needed to borrow it. |
| `ignored_depth_with_handle`       |       712,383 |     148,175,664 | Capability refresh reopened a complete admitted context to read one ignored-depth fact.                          |
| `MainControl::last_node_value`    |       712,383 |     148,175,664 | The same refresh reopened another complete context to inspect the effective tail.                                |
| `CommandContext::page_insertions` |       712,383 |     148,175,664 | The same refresh reopened a third complete context before borrowing insertion rows.                              |
| **Total**                         | **3,200,673** | **665,739,984** | All four values came from one unchanged admission in one synchronous call.                                       |

The candidate creates one stable call-local `CommandContext` for each refresh
episode, updates host capabilities through a shared borrow, and then lends the
same value to the following `CommandProcessor`. Dependency observation still
runs exactly when that processor is admitted. Processor retirement ends only
the episode borrow; it no longer returns or moves the admitted value.
`LinearCommandContext` likewise owns one ordinary value instead of an
`Option` take/restore slot.

All four named baseline owners disappear. One required call-local
materialization remains per refresh episode: 712,383 total 208-byte calls
spread across the ordinary preparation, replay-preflight, and nested display
sites. This is the single capability owner that the facade and refresh now
borrow, not a hidden copy or cache. Across the whole census, 208-byte
`memcpy` rows fall from 7,266,308 calls and 1,511,369,184 bytes to 4,565,348
calls and 949,569,504 bytes, a reduction of 2,700,960 calls and 561,799,680
bytes.

## Architecture simplicity

The default path has fewer representations and handoffs: `from_parts`, both
processor `into_context` layers, the interpreter's live-processor `Option`,
and the cold helper's take/restore protocol are deleted. The admitted value's
existing lifetime remains call-local and explicit. There is no heap
indirection, cache, retained capability summary, special-case workload path,
second facade, or new lifetime owner. Attempt-owned prepared-operation frames
and command resolution are unchanged.

## Exact public-copy and semantic result

Both censuses report zero caller-table and size-table overflow.

| Public API | Base calls | Candidate calls | Call change |    Base bytes | Candidate bytes |  Byte change |
| ---------- | ---------: | --------------: | ----------: | ------------: | --------------: | -----------: |
| `memcpy`   | 36,542,915 |      35,364,432 |  -1,178,483 | 6,120,261,227 |   5,740,579,073 | -379,682,154 |
| `memmove`  |     52,070 |          51,947 |        -123 |     4,795,428 |       4,768,860 |      -26,568 |

The lower aggregate change is reported separately from the exact 208-byte
family because compiler lowering moved unrelated call sizes and immediate
owners between binaries. It does not weaken the direct disappearance of the
four named rows or the exact 208-byte reduction.

Every control, census, and accepted perf row reaches the identical command
work vector
`(20000000, 19913119, 2218327, 6020965, 16785710, 4011)` for fuel charges,
token-frame steps, expanded deliveries, meaning lookups, scanner tokens, and
write expansions. All stop at the same authenticated boundary with status 1,
the same canonical diagnostic, identical empty standard output, and no output
artifact published. Routine completed-run tests cover command, output,
rollback, suspension, mode, and snapshot identity.

## Cycle and allocation confirmation

The baseline zero-loss frame-pointer capture contains 1,528 samples and
17,950,314,510 approximate weighted cycles. The accepted candidate capture
contains 1,663 samples and 19,844,456,765 approximate weighted cycles. Total
cycles are host-variable supporting context, not relabeled as a benefit. The
shared glibc `__memmove_avx_unaligned_erms_rtm` self bucket falls from
1,307,634,119 to 1,142,606,546 weighted cycles, a reduction of 165,027,573
cycles (12.6%). The candidate figure includes 1,094,234,813 cycles with other
libc ancestry and 48,371,733 with `realloc` ancestry; non-self callchain
samples remain outside exact self accounting.

The complete warmed `packed_cutover_gate` passes. Every reported delivery,
backup/replay, stored cursor, macro argument, keyword rollback, primitive
resolution, and destination-directed row retains zero allocation calls and
zero requested bytes.

## Verification

`cargo test -q --tests` passes the complete routine suite. Focused
`tex-command` and `tex-exec` tests pass 241 and 695 unit tests plus their 18,
4, and 22-test integration/fixture groups. `scripts/check.sh` is the authority
for dprint, Biome, rustfmt, and clippy gate status.
