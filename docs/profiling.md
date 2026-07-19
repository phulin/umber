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

## Compact TFM text-run reconstitution

Issue `umber2-q02h.114` replaced immediate scalarization of the existing macro
and physical-source text spans with a TFM-only run state machine. It takes the
pending ligature state once per non-space run, acquires the mode list's
copy-on-write target once, emits nodes directly into that target, and applies
space-factor and paragraph-token accounting in batches. OpenType shaping keeps
the established scalar/source-collecting path. Ordinary TFM runs also no longer
allocate a source-character vector that only shaping consumes.

The first prototype merely wrapped the scalar character loop and regressed its
horizontal owner from 6.77% to 8.10% of whole-run samples; it was removed. A
second prototype buffered emitted nodes in a small vector: reconstitution fell
from 2.17% to 1.60%, but buffer spill made the new path a 1.09% runtime-allocation
owner. Direct mode-list emission removed that allocation. In the final matched
200-run capture, horizontal text delivery fell from 2,111/20,949 weighted
samples (10.08%) to 1,573/20,627 (7.63%), a 24.3% relative reduction. The
ligature/kern state machine itself fell from 454/20,949 (2.17%) to 352/20,627
(1.71%).

The candidate preserved the pinned 97-page, 263,424-byte DVI. All twelve
order-balanced ten-run timing pairs favored it: the baseline averaged 101.181
ms/run and the candidate 99.325 ms/run, a 1.83% whole-Gentle improvement. The
tex-exec test suite and repository format/clippy gate pass.

## Owned alignment-node transfer

Issue `umber2-q02h.118` examined the 22.07% alignment subtree after compact TFM
text runs. Cell, row, and final alignment mode levels were already exclusively
owned when popped, but all three paths cloned their complete node vectors before
math lowering, freezing, or width resolution. They now transfer those vectors
out of the mode level and use the existing owned math-list finalizer.

The matched 200-run sample reduced the complete alignment subtree from
4,553/20,627 samples (22.07%) to 4,470/20,484 (21.82%). Cell packaging fell
from 1.66% to 1.46% of the whole run, and direct node-clone self samples inside
alignment fell from 0.082% to 0.034%. Twelve order-balanced timing pairs
measured 97.032 ms/run for the baseline and 96.706 ms/run for the candidate, a
0.34% improvement; eight pairs favored the candidate. Gentle remained exactly
97 pages and 263,424 DVI bytes.

A broader guarded executor for already-unexpandable alignment templates was
also tested and removed. It eliminated 303 of 172,512 expansion-frame steps,
but alignment increased from 21.94% to 22.23% of whole-run samples and template
replay/get-x attribution remained flat. The common Gentle templates are mostly
macros and scanner-bearing commands; a consequential improvement requires an
invalidation-safe template compiler or a transient alignment representation,
not another per-cell classifier.

## Fused line-width accumulation

Issue `umber2-q02h.115` first replaced each active line-break candidate's copied
start width with an index into an append-only width pool. The representation
preserved break ordering and exact output, but increased `run_pass` from
1,074/20,484 samples (5.24%) to 1,210/20,601 (5.87%) and increased allocation;
the prototype was removed.

The accepted change instead eliminates temporary eleven-field `Widths` values
from the legal-breakpoint scan. Each node now adds directly to the live prefix
or next-line accumulator. When `pdfadjustspacing <= 1`, the scan also skips
font-expansion capacity lookup and arithmetic, which line scoring cannot use in
those modes. Expansion-enabled paragraphs retain the same capacity accounting.

In the matched 200-run capture, the complete `run_pass` subtree fell from
1,074/20,484 samples (5.24%) to 856/20,262 (4.22%), a 19.4% relative reduction.
The old node-width construction path accounted for 1.69% of the baseline whole
run; the fused accumulator accounts for 1.14%, including metric lookup. Twelve
order-balanced ten-run timing pairs measured 98.596 ms/run for the baseline and
97.560 ms/run for the candidate, a 1.05% whole-Gentle improvement; eleven pairs
favored the candidate. Gentle remained exactly 97 pages and 263,424 DVI bytes,
and the tex-typeset and tex-exec test suites pass.

## Owned node-freeze encoding

Issue `umber2-q02h.116` separated production node-freeze work from the
`profiling-stats` payload measurement that scans every compact column. In the
baseline matched capture, `freeze_node_list_owned` occupied 4.16% of Gentle,
but 1.34 percentage points were the profiling-only payload scan. The production
path still traversed each decoded list once for semantic validation and hashing,
again to count and preflight sidecars, and again to encode. Owned sidecar
payloads were cloned during encoding and then immediately dropped when the
source vector was cleared.

The accepted implementation counts and validates sidecar requirements during
the semantic traversal, removing the separate preflight scan. Its owned encoder
then drains the reusable source vector and moves ligature buffers, whatsits,
noads, fractions, and choices directly into compact sidecars. Borrowed freezes
retain the established cloning encoder. Atomic capacity preflight, handle
validation, font sealing, semantic identity, and source-vector capacity reuse
remain unchanged.

Samply failed before recording with macOS error 1100, so the primary comparison
used matched ten-second native `sample` captures of 200-run profiling binaries.
`hpack_owned_with_overfull_rule` fell from 310/7,640 main-thread samples (4.06%)
to 258/7,654 (3.37%), a 17.0% relative reduction. Twelve order-balanced ten-run
timing pairs measured 97.196 ms/run for the baseline and 96.620 ms/run for the
candidate, a 0.59% whole-Gentle improvement; eleven pairs favored the candidate.
A cleanup that forwarded simple owned variants through the borrowed encoder was
rejected after an eight-pair comparison regressed by about 0.34%; the direct
single-dispatch owned match is intentional. Gentle remained exactly 97 pages
and 263,424 DVI bytes.

## DVI-only shipout experiment

Issue `umber2-q02h.119` tested whether plain-DVI execution could bypass the
canonical page-artifact path. Fresh shipout already performs one compact-list
walk that drives the artifact writer and DVI state machine together; there is
no second generic page-model traversal to remove. The canonical artifact is
also the committed page identity used by checkpoints, suffix reuse, replay,
and the public execution result, even when `\pdfoutput=0`. Omitting it would
therefore change engine and incremental semantics rather than specialize an
output formatter.

The post-freeze native sample contained 7,654 main-thread samples. Direct
shipout staging was 681 samples (8.90%), but its visible artifact serialization
leaves were small: `V10NodeListWriter::char` had 17 self samples (0.22%),
`glue` 10 (0.13%), and the remaining artifact writer leaves were individually
below the report's five-sample threshold. The shared emitter itself had 39 self
samples (0.51%), while DVI movement alone had 56 (0.73%). Thus even an invalid
artifact-free ceiling would retain most shipout traversal and DVI work while
removing less than roughly one percent of the whole run. No production
prototype was retained. A useful future output specialization would first need
a different committed-page identity contract; under the current exact
artifact and incremental contract this is not a big compile-time opportunity.

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
