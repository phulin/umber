# Profiling Umber with Gentle

Use the persistent in-process Gentle runner when investigating whole-engine
hotspots:

```bash
scripts/profile-gentle.sh
```

The script builds `gentle-profile` with release optimizations and full debug
information, records 50 measured executions with Samply, and writes the
profile to `target/profiles/gentle.json.gz`. Samply also writes its
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
cargo run --profile profiling -p umber --bin gentle-profile -- \
  --iterations 10 --warmups 1
```

Pass `--checkpoints` to exercise the enabled named-boundary capture path. The
runner consumes every published checkpoint and folds its semantic hash into a
bounded observation instead of retaining snapshots across iterations:

```bash
GENTLE_PROFILE_ITERATIONS=200 scripts/profile-gentle.sh --checkpoints
```

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
