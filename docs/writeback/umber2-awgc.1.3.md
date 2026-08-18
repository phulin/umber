# umber2-awgc.1.3: Integrated Hot-Core Census

## Immutable authority

The integrated measurement used commit
`dd16ecb1e7e6300f922ce8ed432b1ca512acff46`, after the production baseline
and profiling-only census had both landed. The `profiling`-feature binary was
332,373,704 bytes with SHA-256
`a544c41806dbda999c04daddc090d3494fe663a4d5764762067c20406b4ea3aa`.
It was built with one Cargo job; the redirected build log has SHA-256
`e17934b81ffc04e890284d69c37972731f0c329cf91965ad9181db7d94883ee7`.
No production source, fixture, distribution, cache, guard, affinity, or
integrity-policy change is part of this authority.

The source archive, selected `ArXiv.tex`, prepared schema-11 format,
authenticated sparse schema-3 distribution, and ordered 105-key closure retain
the SHA-256 values recorded by
[`umber2-awgc.1.1`](umber2-awgc.1.1.md): respectively
`05a491fc231c85c5827f1dd1b41f80c361f300898d2b3830601c121b0e6d8a2a`,
`816440f61d611fa57cef802e6f372b9337beef1cc4e48e5536d4bad1014ec537`,
`32ae8a46f86ecc3520b48ff6739fa413170f7b34c2263560d7d589abe1466a7b`,
`560ab65f2a4933879b05e47554a9d94434ec1e94ff8f6caa163d26cde7fe35bd`,
and `75d85bb12f8fa5eba0ae2a42daf73fd86c44852ecdc230196455b9aea24565b5`.
The reused cache contained 528 files and retained its complete
`C.UTF-8`-ordered content digest
`37c22368f87e4216cd0759963e0fd2faa9423977094d89f932c05e54a9540b1b`
before and after both rows. There was no acquisition record and no Cargo,
Rust, Umber, perf, or Samply peer before, between, or after them.

## Boundary and process result

The census and sampled rows use the established exact 12,000,000-fuel prefix.
They add only `--profiling-stats` and the profiling feature to the authenticated
pdfLaTeX command, retain unrestricted host affinity `0-23`, unset
`TEXINPUTS`/`TEXFONTS`, and run offline under the unchanged 120-second,
1,536-MiB aggregate-RSS, two-second-TERM guard. The census argument vector has
SHA-256
`0d62501c2cae7dedec0f053863c374776a89b0530ed6d72ddb4b164b52d2107f`.
The guard script remains
`3389d8e5167af44d255cf64bcba9908a1857e2778b9ed3bc2fb6442fc240a063`.

The census row returned the expected typed status 1 with exact command work
`(12000000, 11999815, 1253905, 3485522, 10639582, 1136)` in the documented
order: fuel charges, raw frame steps, expanded deliveries, meaning lookups,
scanner-status tokens, and deferred-write expansions. Full-process time was
11.46 seconds wall, 12.58 seconds user, and 1.22 seconds system. The engine
child was 11.25 seconds wall, 11.00 seconds user, and 0.24 seconds system. Both
observers reported 326,960 KiB peak RSS; the engine incurred zero major
faults. The guard/observer therefore added 0.21 seconds wall without changing
the measured peak. Profiling-counter time is diagnostic and is not a
production latency comparison.

The exact fuel error is the output boundary: neither row published a PDF or
input-record file, and stdout is empty with SHA-256
`e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`.
This matches the uninstrumented full-process authority's absence of published
output, but it does not fabricate equal work: that production row used the
unchanged 100,000,000-fuel command and stopped at the 120.976969-second wall
guard without exposing an exact fuel endpoint. Its 659,172-KiB sampled
aggregate peak and 657,992-KiB `/usr/bin/time` peak remain the sole
production-default wall/RSS authority. The 12M row supplies exact structural
work, not a denominator for the 100M timeout.

## Structural census

The complete machine-readable schema-1 value, including every nonzero episode
length, is frozen in
[`umber2-awgc.1.3-census.json`](umber2-awgc.1.3-census.json). Its SHA-256 is
`751e47dad5ff10af4b8c36131ba3c51b38f6d7f5cd10fd17e9d85c9b29486ff3`.
The independent perf row reproduced every structural field byte-for-byte when
the two elapsed clone timers were excluded; that structural projection has
SHA-256
`24995ad795945edd0cf7cb2710d14d7b584c3398a8dc0a788202e82bc84e0b1b`.

Named owners issued 6,093,009 allocation requests for 952,111,009 bytes:
0.507751 calls and 79.342584 requested bytes per fuel unit.

| Owner                      | Calls     | Requested bytes | Calls / 1M fuel | Bytes / fuel |
| -------------------------- | --------- | --------------- | --------------- | ------------ |
| command-state clone        | 46,074    | 42,771,360      | 3,839.50        | 3.564280     |
| step-snapshot clone        | 21,072    | 6,367,352       | 1,756.00        | 0.530613     |
| delivery and scan          | 4,264,096 | 657,434,887     | 355,341.33      | 54.786241    |
| semantic apply             | 1,220,502 | 199,653,972     | 101,708.50      | 16.637831    |
| weak-value store           | 533,624   | 43,899,840      | 44,468.67       | 3.658320     |
| provenance materialization | 0         | 0               | 0               | 0            |
| evidence publication       | 7,641     | 1,983,598       | 636.75          | 0.165300     |

There were 5,232 attempted episodes and 125,145 attempted operations, a mean
23.919 operations per episode. Length percentiles were p50 5, p90 48, p95
153, and p99 256; 217 attempts reached length 256. Stop reasons were 4,448
internal group-lineage stops (85.015%), 381 committed effects (7.282%), 216
slice limits (4.128%), 186 resource rollbacks (3.555%), and one fuel rollback
(0.019%); every other fixed-vocabulary reason was zero.

Both clone families ran once per episode. `CommandState` cloning presented
4,436,736 logical bytes, exactly 848 bytes/call, in 33,129,938 ns.
`StepSnapshot` cloning presented 40,098,048 logical bytes, exactly 7,664
bytes/call, in 59,122,555 ns. Together their named allocator scopes issued
67,146 requests for 49,138,712 bytes, and the clone timers consumed 92.252493
ms, 0.82% of engine wall.

The reachability graph performed 7,192,938 strong retains, 266,042 weak
retains, and 1,970,183 upgrades, of which 1,933,845 hit. Exact weak indexes
made 213,010 calls, visited 72,394 candidates, performed 35,439 exact
comparisons, and computed 100,926 candidate hashes. Structural provenance
made 181,687 atom intern calls (89,789 hits and 91,898 misses/allocations),
406,916 frame calls (3,960 hits and 402,956 misses/allocations), and 100,897
list calls (45,250 hits and 55,647 misses/allocations). It recorded
6,977,534/7,069,432 atom retains/releases, 498,738/901,694 frame
retains/releases, and 395,840/451,487 list retains/releases. Reclamation
visited/reclaimed 494,853/302,751 atom slots and 55,646/31,160 list slots.
There were zero raw-origin resolutions, while 1,980,300 list-root resolutions
made 7,959,915 owner comparisons. Hot-core structural-origin materialization
was zero calls and zero hits.

The 131,309 meanings reaching main-control reswitch were 115,688
unexpandable primitives (88.104%), 6,550 relax commands (4.988%), 5,031
characters (3.831%), 4,038 registers or parameters (3.075%), and two font
commands; every remaining exhaustive family was zero. Phase entries were
5,232 step snapshots, 125,145 delivery/scans, 124,958 semantic applies,
124,958 evidence publications, and 125,145 barrier decisions. The profile
restored one 2,030,553-byte format containing 317,031 token entries, 19,130
macros, 22 glue entries, and one node through 15 validation passes, 41 support
copies, and 29,312 explicit restore allocations.

## Zero-loss CPU attribution

The independent command used `perf record -F 199 -e cycles:u --call-graph fp`.
It captured 2,151 samples over 10,783.402 ms, reported zero lost samples, and
represented 25,612,011,064 weighted user cycles. For a repeatable disjoint
attribution, each sample goes to the nearest recognized application frame in
its call chain; a sample with no such frame remains in the explicit runtime
bucket. Each sample and its complete weight is assigned once.

| Disjoint owner                                 | Weighted cycles    | Percent    |
| ---------------------------------------------- | ------------------ | ---------- |
| startup, resource, and format                  | 1,575,731,933      | 6.152316%  |
| command delivery, expansion, and scanning      | 7,608,823,684      | 29.708029% |
| semantic execution and modes                   | 496,825,578        | 1.939815%  |
| state, ownership, provenance, and identity     | 8,821,807,770      | 34.444026% |
| typesetting and fonts                          | 0                  | 0%         |
| output publication                             | 0                  | 0%         |
| unresolved runtime, allocator, copy, hash, CLI | 7,108,822,099      | 27.755814% |
| **Total**                                      | **25,612,011,064** | **100%**   |

Zero typesetting/output samples are real phase evidence: this exact prefix is
still dominated by loaded-format LaTeX expansion/scanning and does not publish
output. Flat self-time was led by `memmove` at 10.31%, control-sequence-aware
`get_next` at 6.35%, stored-token delivery at 3.36%, the raw delivery driver
at 3.29%, and SHA-256 compression at 2.53%. The profiling allocator itself
was 1.48% self, another reason not to call this production timing.

The local raw evidence is under `target/umber2-awgc.1.3`. Its 48-file manifest
has SHA-256
`840f1f5484456dcf1a0115269ff671600b19e3cf170eb73bfef19ca743d01946`;
the perf data and disjoint attribution have SHA-256 values
`f8886b9570b2bbc999468965cc1d2815ed50f11710194296882ee5ae4a8f7f75`
and `c488f019c6e6d056d3023fd61204f828f1f7f5a7053249a3c6e6d9234817cc8c`.

## Promotion budgets

Every later child repeats the same pins and exact work/output identity before
claiming a measured improvement. A generic 15% fixed-prefix promotion requires
at most 21,770,209,404 weighted cycles, 277,916 KiB peak RSS, 5,179,057 named
allocation calls, or 809,294,357 named requested bytes. On the unchanged
100M production command, the corresponding baseline RSS threshold is 560,296
KiB. A child may instead establish its named necessary structural invariant,
as the architecture permits, but must not relabel a changed-work prefix as a
performance win.

| Child | Required owner budget or structural invariant                                                                                                                                                                                                                                        |
| ----- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `.2`  | Fixed-size marks allocate zero after warmup; retire the 67,146 clone-scope requests and 49,138,712 requested bytes from the ordinary path.                                                                                                                                           |
| `.3`  | Packed input/macro delivery performs zero per-token retains, upgrades, weak-index lookups, content hashes, or allocation; reconcile against the 4,264,096 delivery/scan requests and every weak-graph/index count above.                                                             |
| `.4`  | Group entry/exit is no longer an episode stop: `internal_group_lineage` falls from 4,448 to zero; resource rollback stays narrow and proportional to changed cells.                                                                                                                  |
| `.5`  | The persistent fused loop removes the universal scanned-step seam and ordinary semantic-apply allocation; reconcile against 1,220,502 requests/199,653,972 bytes and the 29.708% command plus 1.940% executor CPU owners.                                                            |
| `.6`  | Unobserved evidence and structural provenance work compiles out of ordinary delivery: materialization remains zero, while frame/list allocation, 1,980,300 list resolutions, 7,959,915 owner comparisons, and 7,641 evidence-publication allocations move to explicit cold barriers. |
| `.7`  | The single canonical core deletes all legacy hot representations, completes the full pinned paper in at most 20 seconds and 150 MiB, and preserves exact semantic/output identities; all ordinary post-warmup per-token allocation/ownership/hash counters are zero.                 |

These budgets are ceilings, not permission to trade correctness channels.
Exact state, effects, artifacts, PDF, transcript, diagnostics, fuel, rollback,
and incremental boundaries remain mandatory at every promotion.
