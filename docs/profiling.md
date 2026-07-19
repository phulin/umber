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

## Incremental compact-node measurement

Issue `umber2-2xrt` found that the `profiling-stats` peak-memory observer was
changing the algorithm it measured. Every compact-list append called
`payload_bytes`; that routine rescanned all previously accumulated ligature and
whatsit heap payloads. Repeated append therefore made profiling measurement
quadratic in accumulated sidecar rows. It was the largest self-time owner in
the post-freeze capture at 756/7,654 main-thread samples (9.88%).

Compact storage now maintains exact logical and retained totals for nested
ligature and whatsit allocations as rows are appended, compact-copied, or
rolled back. The ordinary fixed set of column capacities remains a bounded
calculation, and detailed peak columns retain the same values. The matched
ten-second native sample reduced `payload_bytes` to 70/7,630 samples (0.92%), a
90.7% relative reduction. Twelve order-balanced ten-run timing pairs all
favored the change: the profiling baseline averaged 96.527 ms/run and the
candidate 87.527 ms/run, a 9.32% whole-Gentle improvement. The production
feature set adds no accounting fields or append work. Gentle remained exactly
97 pages and 263,424 DVI bytes, and profiling measurement tests cover borrowed,
owned, compact-copy, and rollback accounting.

## Batched compact-promotion copy experiment

Issue `umber2-0kij` tested maximal runs of inline compact words (characters,
kerns, ordinary glue, penalties, math boundaries, directions, and styles) in
`NodeStorage::append_compact`. The candidate copied each run's words and
diagnostic origins with two bulk slice extensions while preserving the exact
sidecar preflight and child-patch path.

The matched native samples reduced `append_compact` self time from 144/7,630
samples (1.89%) to 103/7,588 (1.36%), but system `memmove` remained essentially
flat at 267 versus 270 samples. Twelve order-balanced timing pairs were also
flat to slightly adverse: 87.371 ms/run baseline versus 87.407 ms/run candidate,
with five pairs favoring the candidate. The prototype was removed. Per-word
tag dispatch is not the promotion roofline; reducing the 3,057 promotions and
202,149 source words copied by a cold Gentle run is the higher-leverage target.

## Survivor promotion escape census

Issue `umber2-eeka` attributed the promotion volume before attempting
cross-root structural sharing. One cold Gentle run performed 3,057 promotions
(1,872 fresh and 1,185 recycled), visited 16,798 source lists, and copied
202,149 compact words in 4.22 ms of feature-gated promotion timers. Only 11,875
words and 3,548 lists came from existing survivor roots; 190,274 words (94.1%)
and 13,250 lists came directly from the epoch arena. Avoiding copies of
already-immutable survivor subgraphs therefore has a ceiling of roughly 5.9%
of promotion volume, well below a consequential whole-run gain.

A stronger lazy-escape prototype exposed the governing representation
constraint before benchmarking: environment box slots and their unified
rollback journal intentionally encode a survivor span in one 64-bit word. A
live epoch handle includes timeline identity and cannot be stored in that word;
accepted shipout also reclaims the post-mark epoch suffix. Supporting borrowed
epoch values would require widening/redesigning every box slot and undo record,
then promoting or pinning all live references at each reclamation boundary.
That broad state-layout change is not justified by the measured promotion
ceiling, so the prototype was removed. The retained census separates epoch and
survivor source words/lists for future profiles. Directly constructing a
survivor from the owned box payload, while keeping the existing register and
rollback representation, is the narrower higher-value follow-up.

## Non-macro decoded-meaning cache experiment

Issue `umber2-qxh1` added a 64-entry direct-mapped cache for decoded control-
sequence meanings outside immutable macro replay sites. Entries were guarded
by the same store owner and monotonic meaning-write generation as the existing
site cache, so local/global writes, group restoration, and rollback remained
exact. Gentle store lookups fell from 54,483 to 43,916 per run (19.4%), with
the pinned 97 pages and 263,424 DVI bytes unchanged.

The lookup reduction did not reduce sampled decode attribution reliably and
the extra guard/slot probe increased `resolve_meaning_inner` self time. A first
interleaved block was contaminated by host contention; a repeated twelve-pair
block had stable medians of 86.251 ms/run baseline and 86.760 ms/run candidate,
with only one pair favoring the cache. The prototype was removed. At this
working-set locality, decoding the packed meaning is cheaper than probing a
second cache; future expansion gains should eliminate higher-level token or
meaning requests rather than memoize this leaf.

## Dense survivor-promotion remap experiment

Issue `umber2-2plv` tested whether the epoch-heavy promotion workload should
use direct slot indexing instead of hashing every source list. A cold Gentle
run copies 94.1% of its promoted words and 78.9% of its source lists from the
epoch arena, so the candidate kept a reusable dense epoch-slot remap and used
the hash table only for survivor sources. In a matched native sample,
`BuildHasher::hash_one` fell from 23 of 7,630 main-thread samples (0.30%) to 12
of 6,712 (0.18%), while the inclusive promotion share fell from 2.33% to
2.07%. This confirms that the data structure removed the intended local work.

It was nevertheless slower end to end. Twelve alternating ten-run Gentle
pairs had medians of 85.938 ms/run for the baseline and 86.366 ms/run for the
candidate, a 0.50% regression, with only three pairs favoring the dense map.
Growing, clearing, and touching the extra vectors costs more than hashing the
roughly 16,798 source lists at this scale, so the prototype was removed.

The profiling runner now reports a feature-gated compact-node append census to
guide larger storage changes. One measured Gentle run performs 33,339 append
calls for 419,797 words, appends rows to 10 of the 14 sidecar tables, triggers
10,608 retained-vector capacity growth events, and grows retained payload
capacity by 9,512,080 bytes. Per-column attribution assigns 3,662 events each
to words and origins and 2,543 to packed box rows; every other sidecar column
combined accounts for only 741 events (7.0%). This rules out the 14-table shape
as the principal allocation cause.

Path attribution then assigns 3,646 word growths, 3,646 origin growths, and
2,531 box growths to survivor compact copy. In other words, nearly all of the
visible growth is the cost of fresh per-root survivor buffers. However, the
matched whole-run sample places complete promotion at only about 2.3% of
Gentle, so even a zero-cost promotion redesign cannot be a large end-to-end
swing. Chunked or detachable storage remains the principled fix if promotion
becomes a larger workload, but the current optimization loop should first
reduce expansion, line-breaking, and allocator call volume with higher sampled
ceilings.

## Invariant-fast traced-token decoding

Issue `umber2-ffoh` removed redundant validation from the semantic traced-token
decode used throughout expansion. `TracedTokenWord` has a private
representation and its public constructor packs an existing `Token`, so live
engine words already guarantee a valid two-bit kind, catcode discriminant,
Unicode scalar, parameter slot, and frozen-token payload. The checked
`token()` API remains available for test-only raw encodings and validation;
the hot `semantic_token()` path directly decodes the established invariant.

In matched ten-second native samples, checked token decoding plus its expansion
wrapper fell from 167 of 7,602 main-thread samples (2.20%) to 60 of 7,653
(0.78%), a 64.4% relative reduction and 1.42 percentage points of the whole
run. An initial interleaved timing block encountered severe host contention,
with later processes jumping from about 86 ms/run to 132--217 ms/run, and was
discarded. After conditioning both binaries, twelve alternating ten-run pairs
measured medians of 86.622 ms/run baseline and 85.353 ms/run candidate, a
1.46% whole-Gentle improvement; all twelve pairs favored the candidate. Output
remained exactly 97 pages and 263,424 DVI bytes.

## Alignment physical-text batching experiment

Issue `umber2-g2zs` tested whether the large alignment subtree could reuse the
physical-source horizontal text path. The prototype advanced a retired
u-template to the cell body exactly and admitted only directly backed
`Letter`, `Other`, and `Space` tokens; braces, tabs, control sequences, active
characters, superscript notation, tracing, and provenance seams remained on
ordinary alignment interception.

Gentle exposed only 45 additional spans containing 409 tokens, 0.24% of the
172,512 expansion-frame steps. The matched native sample left
`get_x_token_with_context_inner` effectively flat at 2.85% baseline and 2.79%
candidate, while added span probing and TFM delivery absorbed the removed
scalar calls. The exact 97-page, 263,424-byte output was preserved, but the
prototype was removed without wall-clock promotion because the primary
profile established a negligible ceiling. Alignment's roughly 15% inclusive
subtree is dominated by template commands, box construction, and nested
dispatch; a large gain there requires compiled/reusable template semantics,
not another physical-text run path.

## Alignment u-template operation census

Issue `umber2-gxha` tested the remaining compiled-template hypothesis by
counting the commands delivered while each u-template is active. One cold
Gentle run replayed 1,339 templates but delivered only 3,490 operations, an
average of 2.61 operations per replay and 2.02% of the run's 172,512 expansion
frame steps. The stream contained 431 character tokens, 644 relax commands,
342 font commands, 1,717 other unexpandable primitives, and 356 commands with
other meanings. Only 484 operations were the simple `hfil`, `hfill`, `hss`, or
`hfilneg` glue primitives that a narrow no-scan executor could cheaply
specialize.

The census rejects a guarded u-template compiler as a large Gentle win. Its
absolute zero-cost ceiling is already low-single-digit, while a correct cache
must still guard meaning changes, reproduce macro expansion and scanner
interleavings, preserve origins and tracing, dispatch the nontrivial commands,
and retire the alignment marker. The feature-gated counters remain available
to detect a different workload with longer or simpler templates; no production
fast path was added.

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
