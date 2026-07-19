# Profiling Umber with Gentle

Use the persistent in-process Gentle runner when investigating whole-engine
hotspots:

```bash
scripts/profile-gentle.sh
```

The script builds `gentle-profile` with release optimizations, full debug
information, and the compile-time `profiling-stats` instrumentation. It records
50 measured executions with Samply and writes the profile to
`target/profiles/gentle.json.gz`. Override the run counts and output path with:

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
  --features profiling-runner -- \
  --iterations 10 --warmups 1
```

The runner requires the same external inputs as Gentle conformance. Populate
them with `scripts/setup-conformance-tests.sh` if necessary. At startup it
loads `gentle.tex`, `plain.tex`, `hyphen.tex`, and the available Computer Modern
TFM files into a memory-backed `World`. Seeded bytes are structurally shared by
fresh runs; measured iterations include ordinary engine opening, hashing,
initialization, expansion, execution, shipout, and final DVI generation without
repeated host-file staging. The runner verifies output against the pinned
Gentle fixture.

## Checkpoint and incremental modes

Pass `--checkpoints` to exercise named-boundary capture. The runner consumes
each published checkpoint and folds its semantic hash into a bounded
observation instead of retaining snapshots across iterations:

```bash
GENTLE_PROFILE_ITERATIONS=200 scripts/profile-gentle.sh --checkpoints
```

Pass `--incremental-edit` to measure one persistent session across the pinned
five-edit sequence. Adjacent disabled/enabled samples alternate AB/BA order, so
the iteration count must be even. Every policy and revision is checked against
a fresh cold compile for exact DVI and named-boundary schedule equivalence.

```bash
cargo run --profile profiling -p umber --bin gentle-profile \
  --features profiling-runner -- \
  --repo-root /path/to/umber2 --incremental-edit \
  --iterations 6 --warmups 1
```

The sequence separates pagination-changing slow edits, a cross-generation
interaction edit, a height-preserving suffix-adoption edit, and a line-breaking
dependency change. The summary reports paired latency, one-time priming,
boundary equivalence, the actual matched named convergence boundary, page
reuse, replay coverage, cold fallback, retained history, and incremental-to-cold
ratios. Suffix adoption does not require a particular boundary kind: the gate
requires disabled and enabled policies to match each other and remain exactly
cold-equivalent in DVI bytes and boundary schedule.

For path-isolated checks, `--incremental-path slow` exercises the
pagination-changing edit, `fast` the contained equal-width substitution, and
`neutral` a comment-only edit whose DVI must remain identical. Repeat a path
under `--memo-layers none` and `paragraph` in alternating command order when a
focused gate must remain usable independently of the composite sequence.

Use the build without `profiling-stats` for release latency. Its summary prints
`profiling_stats=false` so an attributed run cannot be mistaken for a release
comparison. Rebuild with `--features profiling-runner,profiling-stats` only
when named phases and identity counters are needed to explain a path.

To isolate cold recording overhead, repeat fresh session priming under one
explicit policy:

```bash
GENTLE_PROFILE_ITERATIONS=100 \
  scripts/profile-gentle.sh --cold-memo-layers paragraph
GENTLE_PROFILE_ITERATIONS=100 \
  scripts/profile-gentle.sh --cold-memo-layers disabled
```

`disabled` is the no-runtime control. `none` keeps the memo runtime active with
all recording layers off. Select explicit recording layers with
`--memo-layers`; accepted values are comma-separated
`pretolerance,paragraph,page,shipout`, `all`, and `none`.

For a direct marginal comparison, also pass `--baseline-memo-layers`. Both
policies then run in the same alternating loop and report candidate minus
baseline:

```bash
cargo run --profile profiling -p umber --bin gentle-profile \
  --features profiling-runner -- \
  --repo-root /path/to/umber2 --incremental-edit \
  --iterations 6 --warmups 2 \
  --baseline-memo-layers paragraph \
  --memo-layers pretolerance,paragraph
```

## Stabilization replay gate

Use `--stabilization-replay` for the generated-input slow path. The runner
alternates sixteen generations of one externally supplied reference width over
an unchanged Gentle root. Disabled and paragraph-recording sessions run in
balanced AB/BA order; every accepted DVI is compared between policies. The
receipt includes initial-history and per-pass mean/median latency, pass count,
paragraph lookups/hits/validation misses, reexecuted bytes, retained paragraph
bytes, and the full-session paired delta:

```bash
cargo run --release -p umber --bin gentle-profile \
  --features profiling-runner -- \
  --stabilization-replay --iterations 4 --warmups 1 \
  --memo-layers paragraph
```

Measure the current default WebAssembly editor's linear-memory growth after
building the package. Node must expose garbage collection so the post-disposal
observation is explicit:

```bash
scripts/build-wasm-package.sh
node --expose-gc scripts/measure-wasm-editor-memory.mjs
```

The WASM surface intentionally has no paragraph-recording switch while the
activation gate remains closed. Pair its current-default memory observation
with the native candidate's exact retained paragraph-byte charge; do not infer
that disposal shrinks WebAssembly linear memory, whose pages only grow.

## Interpreting incremental counters

Each layer reports lookups, hits, inserts, evictions, retained bytes, and
misses split into not attempted, ineligible barrier, key miss, first validation
failure, eviction, and detached import failure. Barrier reasons and the first
failing dependency family are separate. Record, lookup, validation/key
construction, and import/mount timing are independent buckets. Accepted-history
metadata is reported separately from detached-cache retention.

`commands_reexecuted` counts tokens reaching scalar main-control dispatch;
`tokens_reexecuted` also includes expansion and scanning below main control.
`commands_skipped` counts recorded main-control deliveries bypassed by accepted
paragraph replay. Compare `commands_reexecuted + commands_skipped` between
policies when evaluating saved interpreter work.

Timing samples taken during thermal pressure or unrelated host contention are
not admissible. Use balanced paired runs, report means and medians, preserve the
cold-output checks, and confirm an apparent win with a separate optimized run.
Detailed historical measurements and rejected experiments remain available in
Git history rather than in this operational contract.

## Guarded macro-command block experiment

Issue `umber2-q02h.117` tested a horizontal main-control block over the only
commands that can be consumed ahead without changing input-scanning order:
ordinary characters, `CharGiven`/`CharToken`, font selection, and `\relax`.
Alignment, tracing, degraded provenance, zero expansion fuel, expandable
meanings, and every command that scans or mutates subsequent input remained on
the scalar path. A profiling-only census showed why this guard is narrow:
macro-replayed unexpandable commands were led by `\hbox` (6,250), `\setbox`
(2,802), `\char` (2,425), prefixes (1,726), `\unhbox` (1,528), catcode writes
(1,509), penalties (1,258), skips (1,207), boxes (1,059), and kerns (1,017).

The candidate preserved the pinned 97-page, 263,424-byte DVI and removed 766
of 172,512 expansion-frame steps (0.44%). In matched 200-run Samply captures,
however, `get_x_token_with_context_inner` increased from 3,177/20,949 samples
(15.17%) to 3,287/21,074 (15.60%); the new classified-span probe owned another
27 samples (0.13%). Twelve order-balanced timing pairs leaned favorable by
about 0.62 ms/run, but sample attribution is the primary decision evidence on
this contended host. The prototype was removed. A broader block requires a
real macro compiler/deoptimizer capable of preserving arbitrary scanner and
meaning-write interleavings; adding another main-loop probe is not supported.

## Analyze a capture

Use the repository analyzer for a repeatable text report:

```bash
scripts/analyze-profile.sh
scripts/analyze-profile.sh --top 40 target/profiles/gentle.json.gz
```

The report ranks self time and recursion-deduplicated inclusive time. It also
attributes runtime self samples to the nearest Umber frame, making allocator
and memory-operation costs visible without losing their application caller.
Percentages use Samply sample weights.

For a focused question, restrict the report to stacks beneath a function:

```bash
scripts/analyze-profile.sh \
  --subtree drain_pending_output \
  target/profiles/gentle.json.gz
```

The subtree report adds immediate callees, immediate callers, and nearest
non-runtime application callers. Samply normally writes
`gentle.json.syms.json` beside `gentle.json.gz`; the analyzer discovers that
sidecar automatically. Pass `--symbols PATH` for a sidecar elsewhere,
`--thread TEXT` or `--app TEXT` when selection is ambiguous, and `--json` for
machine-readable output. If no sidecar exists, unresolved addresses are
reported rather than guessed.

Compiler inlining can make broad frames such as `main` or `run` dominate an
inclusive ranking. Self time is unaffected; use a named subtree when comparing
the internal costs of a subsystem.
