# Profiling Umber with Gentle

Use the persistent in-process Gentle runner when investigating whole-engine
hotspots:

```bash
scripts/profile-gentle.sh
```

The script builds `gentle-profile` with release optimizations, full debug
information, and the compile-time `profiling-stats` instrumentation, records
50 measured executions with Samply, and writes the profile to
`target/profiles/gentle.json.gz`. Samply also writes its
presymbolication sidecar next to the profile when supported. Override the run
counts and output path with:

```bash
GENTLE_PROFILE_ITERATIONS=100 \
GENTLE_PROFILE_WARMUPS=2 \
GENTLE_PROFILE_OUTPUT=/tmp/gentle.json.gz \
scripts/profile-gentle.sh
```

Extra arguments are forwarded to the runner. Run the optimized workload
without Samply when checking timing or setup:

```bash
cargo run --profile profiling -p umber --bin gentle-profile \
  --features profiling-stats -- \
  --iterations 10 --warmups 1
```

Pass `--checkpoints` to exercise the enabled named-boundary capture path. The
runner consumes every published checkpoint and folds its semantic hash into a
bounded observation instead of retaining snapshots across iterations:

```bash
GENTLE_PROFILE_ITERATIONS=200 scripts/profile-gentle.sh --checkpoints
```

Pass `--incremental-edit` to measure a fixed semantic prose edit 20% through
`gentle.tex`. The runner interleaves memo-disabled incremental execution,
memo-enabled incremental execution, and a cold compile of the edited document;
it reports latency and reuse/memo telemetry and requires both incremental modes
to produce the cold compile's exact DVI bytes:

```bash
cargo run --profile profiling -p umber --bin gentle-profile -- \
  --repo-root /path/to/umber2 --incremental-edit --iterations 5 --warmups 1
```

The fixed edit inserts 1,792 words into one paragraph beginning 19.66% through
the source. It deliberately changes both line and page breaking: the pinned
document grows from 97 to 98 pages, so the later 84-page suffix cannot be
adopted. A five-sample optimized run on 2026-07-15 measured 3.986 seconds mean
(2.940 seconds median) with memoization disabled, 7.304 seconds mean (7.222
seconds median) with memoization enabled, and 2.875 seconds mean (2.740 seconds
median) for a cold compile of the edited document. Memoization was therefore
83% slower by the means and 146% slower by the medians. It made 7,140 lookups
but only 385 hits, reached 67,008,455 retained bytes, and evicted 6,475 entries.
Both incremental modes produced the cold compile's exact 98-page, 278,000-byte
DVI. This is an intentional page-divergence stress case, not the expected case
for suffix reuse.

After stable pre-delivery paragraph anchors landed, an independent five-sample
rerun on 2026-07-16 preserved that exact DVI and raised the downstream eligible
paragraph result to 121 of 129 candidates (93.8%). Memo-enabled execution still
lost to memo-disabled execution: 9.446 versus 6.409 seconds by the means and
8.562 versus 2.967 seconds by the medians. The general detached cache retained
66,899,304 bytes and evicted 6,721 entries; page episodes made 5,378 lookups for
30 hits on the deliberately pagination-shifting edit. These observations do
not close the performance gate. Complete per-layer miss reasons and separated
record, lookup, validation, and import timings are required before attributing
the paragraph layer's own cost; that instrumentation is tracked by
`umber2-vfqs.16`. Removal of the standalone expansion-episode and pretolerance
caches remains tracked by `umber2-vfqs.17`.

The runner requires the same external inputs as Gentle conformance. Populate
them with `scripts/setup-conformance-tests.sh` if necessary.

At startup, the runner reads `gentle.tex`, `plain.tex`, `hyphen.tex`, and the
available Computer Modern TFM files into a memory-backed `World`. Seeded input
bytes are structurally shared by every fresh run. One warm-up and all measured
iterations execute in the same process without temporary directories, corpus
copies, or host file reads. Each iteration still includes normal engine input
opening and hashing through `World`, engine initialization, expansion,
execution, shipout, and final DVI generation. After sampling, the runner checks
the final DVI against the warm-up result.

The optimized runner must produce 97 pages and a 263,424-byte DVI for the
current pinned Gentle corpus. That byte count is also the size of
`tests/corpus/e2e/gentle.expected.dvi`; the strict conformance test remains the
authoritative byte-for-byte check. Profiles captured before the macro-text
release-path repair consumed lexer spans from inside a `debug_assert!`, so the
side-effecting character append was compiled out and those runs produced an
incorrect 257,304-byte DVI. Do not compare their absolute timings or hotspot
weights with corrected captures. The first corrected scalar 200-run capture is
`/tmp/gentle-q50-correct-scalar-baseline.json.gz` (92.351 ms/run on the capture
machine).

The ordinary end-to-end Gentle conformance test remains the correctness and
host-workflow measurement. This profiling runner deliberately removes its
repeated staging, oracle reads, artifact writes, and temporary-directory
cleanup so those operations do not obscure engine hotspots.

## Expansion meaning-site cache evidence

The expansion meaning-site cache is guarded by the owning `Stores` identity
and a monotonic meaning-write generation. A 200-iteration corrected Gentle run
after the expanded-replacement span fix produced 97 pages and 263,424 DVI
bytes, with 20,240 cache hits and 57,307 misses at guarded macro-body sites
(26.1% hits). Profiling-only invalidation counters over the warm-up plus 200
measured runs recorded 448,431 local meaning writes, 21,507 global meaning
writes, and 2,217,432 conservative group-exit invalidations.

Returning one `meaning_changed` bit from Env group restoration to the owning
`Stores` boundary lets empty and non-meaning groups retain valid entries while
both group-exit paths still invalidate whenever their journal restores or
compacts a meaning cell. The corrected 200-iteration rerun increased reuse to
40,953 hits against 36,594 misses (52.8% hits) and reduced group-exit
invalidations to 91,857, while retaining the same 97 pages and 263,424 bytes.
Local/global writes, rollback, owner isolation, and both group-exit paths have
focused invalidation coverage; debug cache hits also compare against the live
aggregate meaning.

After moving the cache and current replay-site state from `tex-lex::InputStack`
to the persistent `tex_expand::ExpansionContext`, a corrected 200-iteration
run retained 40,925 hits and 36,650 misses, completed at 118.061 ms/run on the
capture machine, and again produced 97 pages and 263,424 bytes. The lexer now
delivers only semantic-free macro replay-site metadata; a compile-fail boundary
test prevents `InputStack` from regaining meaning resolution.

The conditioned `BOOB` plus five `BOOBOBBO` paired comparison was noisy but
flat: refined versus conservative raw means were 134.191 and 134.350 ms/run,
medians were 118.299 and 117.795 ms/run, and means after excluding the two
greater-than-200-ms host-contention outliers on each side were 119.857 and
119.383 ms/run. The selective policy is retained because it removes needless
invalidation through a small exact journal-owned signal, materially improves
cache reuse, and shows no meaningful throughput regression; the cache itself
remains justified by the corrected 52.8% guarded-site hit rate.

## Physical-source text-run evidence

Horizontal main control now has a guarded physical-source path alongside the
existing immutable macro-replay span path. It accepts only directly backed
`Letter` and `Other` scalars under their current catcodes; all lexer-semantic,
provenance, tracing, alignment, and source-frame boundaries remain scalar.
A corrected 200-iteration Gentle run produced the pinned 97 pages and 263,424
DVI bytes, with 20,436 accepted runs containing 89,522 tokens (4.381 tokens per
run) from 48,407 source-path probes, a 42.2% accepted-run rate.

The same conditioned `BOOB` plus five `BOOBOBBO` comparison was throughput
flat across 22 ten-iteration samples per binary. The pre-change baseline mean
and median were 112.247 and 111.843 ms/run; the source-run candidate measured
112.263 and 112.003 ms/run, a -0.014% mean change. The path is retained as an
exact, localized reduction in per-token expansion delivery rather than a
claimed end-to-end speedup. Focused tests cover dynamic catcode and UTF-8
cursors, `^^` and alignment deoptimization, tracing, summary restoration, and
the precise provenance seam; strict Gentle remains byte-identical.

## Analyze a capture

Use the repository analyzer for a repeatable text report instead of manually
expanding stacks in the Samply UI:

```bash
scripts/analyze-profile.sh
scripts/analyze-profile.sh --top 40 target/profiles/gentle.json.gz
```

The report ranks self time and recursion-deduplicated inclusive time. It also
separates runtime self samples by library and attributes them to the nearest
Umber frame, which makes allocator and memory-operation costs visible without
losing their application caller. Percentages use Samply sample weights rather
than assuming every sample has weight one.

For a focused question, restrict the report to stacks beneath a function. The
subtree report adds the function's immediate callees, immediate callers, and
nearest non-runtime application callers. This assigns allocator or memory
runtime to the engine operation above it while still showing both the share
within the subtree and the share of the complete capture:

```bash
scripts/analyze-profile.sh \
  --subtree drain_pending_output \
  target/profiles/gentle.json.gz
```

Samply normally writes `gentle.json.syms.json` beside `gentle.json.gz`; the
analyzer discovers that sidecar automatically and uses its exact address map,
including inline frames. Pass `--symbols PATH` for a sidecar elsewhere. If no
sidecar exists, already-symbolized profile names remain useful and unresolved
addresses are reported explicitly rather than guessed. Use `--thread TEXT` or
`--app TEXT` when automatic thread or application-library selection is
ambiguous, and `--json` for machine-readable output.

Compiler inlining can make broad entry-point frames such as `main` or `run`
dominate a whole-profile inclusive ranking. Self time is unaffected; use a
named subtree when comparing the internal costs of a subsystem.

## Checkpoint optimization evidence

Measure checkpoint changes with both a 200-iteration Samply capture and a
thermally conditioned, interleaved wall-clock comparison. Use one warm-up and
ten measured runs per invocation; condition with `BOOB`, then measure
`BOOBOBBO` five times. Compare checkpoint-enabled binaries first and run the
disabled control only for a candidate that improves enabled timing. This
pairing is necessary because sub-percent Samply movements on thermally
constrained machines have repeatedly failed to predict throughput.

After the page forest and mutable tail gained structural lazy projections, a
200-run Gentle capture placed the complete `EngineSession::publish` subtree at
about 3.4% of whole-run samples. Its remaining work was fragmented across the
journal (about 0.6%), current page (about 0.6%), input hashing (about 0.4%), and
input summary construction and validation (about 0.4%). Experiments that
removed a sampled clone/drop, recursively composed page-tail roots, fused input
validation with projection, or changed canonical byte decoding were flat or
worse under the paired evidence. Treat those residual entries as attribution,
not independent optimization budgets: moving work between their inline frames
does not by itself demonstrate a faster checkpoint path.

The corrected 263,424-byte workload exposed a larger shared cost below those
components: `StateHasher` previously ran a complete two-multiply SplitMix64
avalanche for every canonical scalar and then finalized with another avalanche.
Checkpoint hash schema version 8 replaces the per-field avalanche with a
target-independent ordered one-multiply recurrence while retaining SplitMix64
as the finalizer. A corrected 200-run checkpoint profile reduced publication
from 3.46% to 2.71% and mean time from 96.846 to 92.641 ms/run. Conditioned
`BOOB` plus `BOOBOBBO` five times confirmed 93.607 to 89.704 ms/run with
checkpoints (4.17%) and 89.847 to 86.762 ms/run without checkpoints (3.43%);
wall-clock deltas were 4.35% and 3.74%, respectively.

## Node-sidecar allocation evidence

After content identity v2 removed the earlier hash hotspot, caller
reconstruction placed `RawVec::finish_grow` at 8.40% of a 200-run default
Gentle profile. The largest coherent owner was compact node storage, including
0.56% directly under the nine-column box-sidecar reserve. Boxes are consumed
as complete values in the current execution, packing, copy, and shipout paths,
so row-packing that one sidecar removed the independent box reserve frame and
reduced `finish_grow` to 7.91%.

Thermally conditioned `BOOB` followed by `BOOBOBBO` five times, with ten
measured runs per process, confirmed the layout change. Default Gentle improved
from 99.187 to 97.657 ms/run (1.54%); checkpoint-enabled Gentle improved from
103.819 to 101.919 ms/run (1.83%). Other sidecars remain columnar because this
evidence applies specifically to complete-box access, not to a general retreat
from compact field scans.

## Direct shipout evidence

The direct-emission refactor removed fresh-path `FrozenShipoutNode` snapshots,
ordinary `PageNode` materialization, and the artifact-byte DVI reparse. A
200-run default capture reduced the complete `shipout_node` subtree from
15.22% to 11.14% of whole-run samples (26.8% relative). Within the candidate,
fused arena emission was 6.47% of the whole run and mutable normalization was
2.58%. The capture still charged 1.05% to a decoded direction prescan; the
accepted implementation subsequently replaced that work with a raw compact-tag
predicate.

Conditioned `BOOB` plus `BOOBOBBO` five times measured 111.963 to 100.468
ms/run by raw default mean. Removing one isolated 258 ms baseline outlier gives
104.277 to 99.315 ms/run (4.76%). A post-tag-optimization checkpoint-enabled
repetition measured 135.664 to 131.800 ms/run (2.85%), with effectively equal
medians of 105.909 and 105.865 ms/run. Host contention produced large paired
outliers, so retain both raw means and medians when comparing this run. All
processes emitted 97 pages and 263,424 DVI bytes; checkpoint runs published
1,108 checkpoints.
