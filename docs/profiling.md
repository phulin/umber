# Profiling Umber with Gentle

## Command-stream repository comparison

The exhaustive command-stream repository comparison is a manual performance
workload, not a native test: its generated Plain, Story, and Gentle traces are
too large for routine gates. Run the user-visible optimized-development
measurement with:

```bash
/usr/bin/time -f 'wall=%e maxrss_kib=%M' \
  cargo run-dev -q -p tex-command-stream --bin tex-command-stream -- \
  --repository . --max-divergences 100000
```

It must remain an exhaustive `DIVERGED` run (exit 1) with its current exact
per-fixture accounting, ordered worklist, grouping, and report text. Attribute
hotspots separately with a symbolized optimized capture; the `profiling`
profile keeps release optimization and full debuginfo:

```bash
cargo build --profile profiling -p tex-command-stream
perf record -F 99 --call-graph fp -o target/profiles/command-stream.perf -- \
  target/profiling/tex-command-stream --repository . --max-divergences 100000
perf report --stdio --no-children --percent-limit 0.2 \
  -i target/profiles/command-stream.perf
```

At the initial `umber2-johp.287` measurement, source-line indexing in
`tex-state` was the dominant cross-subsystem cost (51.29% self samples), with
the tracer's observation translation next (15.86%). Keep improvements scoped
to the attributable owner: file a cross-subsystem issue for a dominant hotspot
outside `tools/tex-command-stream`; do not infer a target from source reading.
Bounded synthetic streams and focused unit tests are the suitable regression
coverage for comparison and translation kernels.

Issue `umber2-dz4x` found that the index itself was already retained with the
rollback-coupled source region, but main control asked to register
the same immutable source before every delivery. `Universe::register_source`
therefore rebuilt and discarded the complete newline index before
`SourceMap` recognized the identical registration. Registration now validates
World record liveness and length as before, then resolves the existing live
descriptor before deriving an index. Generated backings use shared-`Arc`
identity as their common constant-time path while retaining exact byte
comparison for separately allocated equal content. Source-map rollback still
invalidates the registration and its index together.

The focused benchmark is:

```bash
cargo bench --manifest-path benchmarks/tex-state/Cargo.toml \
  --bench state_budgets repeated_source_registration
```

It repeatedly registers one shared 1 MiB generated source after priming its
live source region. This is the bounded reproduction of the command-stream
hot path; full Plain, Story, and Gentle traces remain manual profiling inputs
rather than test fixtures.

The first cache-aware candidate still compared the full shared byte slice and
measured 36.020 µs median. The final shared-backing fast path measured
55.545 ns median and 56.061 ns mean, a 99.8445% reduction. A 29K-sample
`perf -F 999` capture over the focused release benchmark reported
`SourceMap::existing_registration` as the remaining registration kernel and
no samples in `source_line_starts`. The exhaustive report signature this path
must preserve is 61 divergences in 24 root sites: transitions, Plain, and Story
clean; Gentle's first divergence at event 476410; `DIVERGED` exit 1.

Issue `umber2-johp.295` profiled the exhaustive comparison again after that
source-registration fix. A symbolized `perf record -F 199 --call-graph fp`
capture collected 10K cycles samples with none lost. The next dominant symbol
was the cross-crate `ContentIdentity::for_domain` at 33.51% self samples,
followed by the engine-owned `DviPagePlan::clone` at 19.06% inclusive and
7.92% self. The largest `tex-command-stream`-owned kernels were
`events_match` at 0.25% self and `translate_observation` at 0.23% self.

Those tool-owned ceilings are too small to support a credible end-to-end
optimization, so no tracer change was promoted. On the measured base, the
current exhaustive signature was 23 divergences in 9 root sites with
`DIVERGED` exit 1; this supersedes older event totals only for comparisons
based on that commit.

Issue `umber2-johp.308` profiled the bounded five-entry command-stream report
after source-descriptor retention, shared-DVI rollback, and the promoted mode
inverse journal. The repeated workload replayed the two committed
microfixtures while leaving the ungenerated Plain, Story, and Gentle document
entries explicitly unconfigured; it did not trace a full document. A matched
100-run `perf -F 499 --call-graph fp` capture attributed 44.47% self samples to
`memcmp` and another 19.69% to
`CommittedFixture::audit_matrices`. The audit was scanning every serialized
event with a naive byte window for each required semantic fragment.

Each requirement now constructs one `memchr::memmem::Finder` and reuses it
across the event stream. The event boundary remains exact: a match must still
occur wholly inside one canonical event. Twelve order-balanced timing pairs
reduced mean latency from 663.264 ms to 289.168 ms and median latency from
703.325 ms to 305.700 ms; all twelve pairs favored the candidate. The matched
candidate capture reduced the complete substring search to 9.31% self samples
(`searcher_kind_two_way_with_prefilter` plus its AVX2 prefilter), with SHA-256
now the largest self-time owner at 18.32%. Both captures reported zero lost
samples. The five-entry report and committed-microfixture semantic identity
were unchanged.

Use the persistent in-process Gentle runner when investigating whole-engine
hotspots:

```bash
scripts/profile-gentle.sh
```

The script builds `gentle-profile` with release optimizations, full debug
information, and the compile-time `profiling` instrumentation. It records
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
them with `python3 scripts/provision.py worktree .` if necessary. At startup it
loads `gentle.tex`, `plain.tex`, `hyphen.tex`, and the available Computer Modern
TFM files into a memory-backed `World`. Seeded bytes are structurally shared by
fresh runs; measured iterations include ordinary engine opening, hashing,
initialization, expansion, execution, shipout, and final DVI generation without
repeated host-file staging. The runner verifies output against the pinned
Gentle fixture.

## TeX82 diagnostic memory-projection census

A `profiling` build of the `umber` CLI accepts `--profiling-stats` and emits a
process-local `MAIN_MEMORY_PROJECTION` census. The report is owned by the CLI
run scope, so it is printed on exact command-fuel exhaustion as well as normal
completion. This makes bounded endpoints directly comparable without changing
the document, authenticated distribution, cache, or execution guards.

`base_requests`, `base_reuses`, and `full_rebuilds` account for every request
for the live-root allocator base. `dynamic_observations` names scanner-owned
one-word samples. Operation-boundary, cell-root, and box-root fields report
attempts and retained projections. One initial cold construction plus the
reported cache losses account for the full rebuilds in a single-run census.
Separate
`MAIN_MEMORY_PROJECTION_CACHE_LOSS` lines exhaustively name
`operation_boundary`, `timeline_rollback`, `profile_change`,
`cell_root_update`, and `box_root_update`, including owners whose count is
zero. These counters observe derived work only and never enter snapshots,
formats, semantic hashes, or TeX82 high-water values.

Build and run a pinned endpoint with one Cargo job and the normal isolated
resource guard, adding only the profiling feature and report flag:

```bash
CARGO_BUILD_JOBS=1 cargo build --profile profiling \
  --features profiling -p umber --bin umber
target/profiling/umber run --profiling-stats \
  --expansion-fuel 6000000 <the unchanged pinned run arguments>
```

## Main-control hot-core structural census

A `profiling` build of the `umber` CLI also emits one `HOT_CORE_CENSUS`
record when `run --profiling-stats` is requested. The text after that prefix is
a compact schema-2 JSON object, so a census consumer parses the JSON rather
than scraping field positions. The report is owned by the same run-scope guard
as `MAIN_MEMORY_PROJECTION`; normal completion, typed failure, and exact fuel
exhaustion all publish the delta, while a run without `--profiling-stats`
publishes neither line.

All underlying counters are process-local, relaxed-atomic, and monotonic. A
run captures their values at scope entry and publishes a saturating delta at
scope exit. There is no mutable global reset: concurrent or nested diagnostic
code cannot erase another measurement, and counters never enter semantic
state, snapshots, rollback, formats, hashes, checkpoints, fuel, or output.

The same report scope emits `NODE_POOL_STORAGE_CENSUS`. Its separate node and
annex lanes give exact fresh/reuse/release event totals, current and peak live
block counts, current and peak vacant stable-slot counts, and corresponding
live and vacant payload bytes. NodePool last-lineage release empties the exact
64 KiB allocation and keeps it warm for direct incarnation-safe reuse; the
separate owner census excludes that vacant capacity from checkpoint charges.
The counters are
profiling-only scalar updates at allocation and release boundaries; they do
not scan tables, decide liveness, or change ordinary arena retention policy.

The JSON fields have these semantics:

- `allocations` always names `command_state_clone`, `step_snapshot_clone`,
  `delivery_and_scan`, `semantic_apply`, `weak_value_store`,
  `provenance_materialization`, and `evidence_publication`, including owners
  whose count is zero. Schema 2 additionally names
  `interpreter_construction`, `interpreter_borrow`, and `apply_step_clone`;
  these nested scopes remove their requests from the broader phase owner so
  every request still has exactly one owner. `calls` counts `alloc`,
  `alloc_zeroed`, and `realloc`;
  `requested_bytes` adds the requested allocation size, using the new size for
  `realloc`. Deallocation is not a request and is not counted. A thread-local
  nested scope assigns each request to exactly its innermost owner. Allocations
  outside the named current-core regions, including general startup, resource,
  and final output work, are deliberately unowned rather than guessed into a
  bucket.
- `episode_lengths` is a sparse JSON object over the exact inclusive
  `0..=256` operation domain. It records committed and rolled-back attempted
  episodes. `stop_reasons` is exhaustive and includes slice, every semantic
  barrier, named checkpoint, terminal, both internal lineage stops, and typed
  resource, diagnostic, and fuel rollback.
- `clones.command_state` and `clones.step_snapshot` are frozen schema slots.
  After the journaled transaction cutover both must remain exactly zero; any
  nonzero call, elapsed time, logical byte, or corresponding owner allocation
  is a regression.
- `weak_graph` counts strong `Arc` retains, weak retains, and weak-upgrade
  calls/hits in the reachability-owned token, macro, and glue value substrate.
  `weak_index` counts exact-index calls, candidate entries, exact comparisons,
  and weak-candidate content hashes. The existing `PROVENANCE_LIFECYCLE`
  record remains the more detailed owner-specific provenance graph census.
- `provenance_materialization` counts attempts and successes converting a
  compact `OriginId` into a structurally owned `OriginRef`.
- `command_families` classifies every meaning reaching the canonical
  main-control reswitch loop. Prefixes and commands fetched by `ignore_spaces`
  are each counted when they actually reach that loop; expanded-away macros
  remain visible through the profiling-only command-work vector rather than
  being fabricated as dispatches.
- `expansion_opcodes` counts each real macro expansion and every expandable
  primitive by its stable serialized operand and Rust catalogue name.
  `dispatch_opcodes.unexpandable_primitives` does the same for each primitive
  that reaches main control. Zero-count opcodes are retained in the fixed
  counter arrays but omitted from the JSON map. Operand bounds are asserted in
  profiling builds, so extending either primitive catalogue without extending
  the census fails loudly.
- `materializations` counts the command value handed to expansion, the
  universal `ScannedStep`, its `PreparedOperation` wrapper, and the
  `ScannedStep` clone passed to semantic apply. These are structural events,
  not fuel, and exist to prove which DTO seams fused dispatch removes.
- `interpreter.constructions` counts session-owned interpreter construction;
  `interpreter.operation_entries` counts borrow-scoped processor entries over
  that owner. The latter may exceed prepared operations because scanners open
  nested, nonoverlapping processor borrows. Both are monotonic profiling-only
  evidence and never enter the interpreter lifecycle or semantic state.
- `phase_boundaries` retains the historical step-snapshot slot for schema
  comparison, but production must report zero entries there. Delivery/scanning,
  semantic apply, evidence publication, and barrier decision remain active.

The allocator forwarding implementation is isolated in
`crates/tex-state/profiling-allocator`; it is selected only by the existing
`profiling` axis. All call sites, scopes, counters, and the allocator selection
are `#[cfg(feature = "profiling")]`, so the production feature resolution
contains no additional hot-path field, branch, call, allocation hook, or
reference-count operation. The command-work detail fields follow the same
axis: production `CommandFuel` stores only its limit and remaining countdown,
while profiling adds the exact raw-owner, expanded-delivery, meaning-lookup,
scanner-token, and write-expansion census. Publication derives token-frame
steps from the exhaustive raw-owner vector rather than storing a parallel hot
counter.

The pinned integrated authority for later HotCore work is
[`writeback/umber2-awgc.1.3.md`](writeback/umber2-awgc.1.3.md), with its exact
machine-readable baseline in
[`writeback/umber2-awgc.1.3-census.json`](writeback/umber2-awgc.1.3-census.json).
A successor repeats the exact 12,000,000-fuel work/output identity and reports
the complete schema rather than comparing selected favorable counters. CPU
attribution assigns each sample's complete weight to one nearest recognized
application owner, retaining an explicit unresolved-runtime bucket so the
owners remain disjoint and sum to 100%. The separate production-default
100,000,000-fuel wall/RSS guard remains a different boundary and must not be
presented as equal work.

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
under `--memo-layers none` and the relevant remaining pure layer in alternating
command order when a focused gate must remain usable independently of the
composite sequence.

Use the build without `profiling` for release latency. Its summary prints
`profiling_stats=false` so an attributed run cannot be mistaken for a release
comparison. Rebuild with `--features profiling-runner,profiling` only
when named phases and identity counters are needed to explain a path.

To isolate cold pure-cache recording overhead, repeat fresh session priming
under one explicit policy:

```bash
GENTLE_PROFILE_ITERATIONS=100 \
  scripts/profile-gentle.sh --cold-memo-layers page
GENTLE_PROFILE_ITERATIONS=100 \
  scripts/profile-gentle.sh --cold-memo-layers disabled
```

`disabled` is the no-runtime control. `none` keeps the memo runtime active with
all recording layers off. Select explicit recording layers with
`--memo-layers`; accepted values are comma-separated
`pretolerance,page,shipout`, `all`, and `none`.

For a direct marginal comparison, also pass `--baseline-memo-layers`. Both
policies then run in the same alternating loop and report candidate minus
baseline:

```bash
cargo run --profile profiling -p umber --bin gentle-profile \
  --features profiling-runner -- \
  --repo-root /path/to/umber2 --incremental-edit \
  --iterations 6 --warmups 2 \
  --baseline-memo-layers page \
  --memo-layers pretolerance,page
```

## Paragraph replay deletion comparison

The paragraph replay deletion baseline and post-deletion workload identities
are recorded in
[`paragraph_replay_deletion_baseline.md`](paragraph_replay_deletion_baseline.md).
Use `--edit-restart-workload` for the paired synthetic edit cases. Every
accepted DVI is compared with a fresh cold target. The receipt includes
latency, RSS, allocation volume, snapshot cost, convergence, and reexecution
counters. The runner uses the native editor's 64 MiB checkpoint-root budget
and fails if pruning, direct revision ownership, the one-/two-generation
lifecycle, zero replay retention, cold-DVI identity, or the workload's latency
gate fails:

```bash
(cd benchmarks/edit-restart/workloads && sha256sum -c SHA256SUMS)
cargo run --release -p umber --bin gentle-profile \
  --features profiling-runner -- \
  --edit-restart-workload prefix --iterations 6 --warmups 2 \
  --memo-layers none
```

The fixed generated long acceptance is exactly two measured advances after
one warmup. Build outside the runtime guard, then run the already-built
attributed binary under the finite 1 GiB and 1,200-second guard:

```bash
cargo build --profile profiling -p umber --bin gentle-profile \
  --features profiling-runner,profiling
systemd-run --user --scope --quiet -p MemoryMax=1G -p MemorySwapMax=0 \
  /usr/bin/time -v timeout --signal=TERM --kill-after=10s 1200s \
  target/profiling/gentle-profile \
  --edit-restart-workload long --iterations 2 --warmups 1 \
  --memo-layers none
```

The receipt also defines the active cold-relative latency, generic suffix
adoption, and structural retention budgets. A changed workload checksum needs
a new attributed baseline; it must not be compared as though it were the
frozen corpus.

Measure the current default WebAssembly editor's linear-memory growth after
building the package. Node must expose garbage collection so the post-disposal
observation is explicit:

```bash
scripts/build-wasm-package.sh
node --expose-gc scripts/measure-wasm-editor-memory.mjs
```

Do not infer that disposal shrinks WebAssembly linear memory, whose pages only
grow.

## Interpreting incremental counters

Each layer reports lookups, hits, inserts, evictions, retained bytes, and
misses split into not attempted, ineligible barrier, key miss, first validation
failure, eviction, and detached import failure. Barrier reasons and the first
failing dependency family are separate. Record, lookup, validation/key
construction, and import/mount timing are independent buckets. Accepted-history
metadata is reported separately from detached-cache retention.

`commands_reexecuted` counts tokens reaching scalar main-control dispatch;
`tokens_reexecuted` also includes expansion and scanning below main control.

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

## Non-macro decoded-meaning cache experiment

The feature-gated local-write, global-write, group-exit, and rollback
invalidation census belonged to the earlier expansion meaning-site cache. The
cache, its `MeaningCacheGuard`, and its measurement hooks were retired with the
expansion state facade, so `gentle-profile` no longer reports meaning-cache
invalidations. Restoring that census without a guarded cache would measure no
live optimization and would add profiling overhead without an attribution
owner.

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

## Preallocated oracle event decoding

Issue `umber2-q02h.121` profiled the complete optimized command-stream
repository comparison, including the generated Plain, Story, and Gentle
traces. A 1,661-sample `perf -F 199 --call-graph fp` capture lost no samples.
Mandatory SHA-256 fixture validation was the largest self-time owner at
15.82%; `memmove` was next at 12.89%, driven in part by repeated growth of the
multi-million-entry decoded event vector.

The decoder now counts line delimiters in its already-resident JSONL input and
allocates the exact event capacity before canonical decoding. Hash validation,
canonical re-encoding checks, sequence validation, checkpoint and observation
values, and fixture identity remain unchanged. A matched candidate capture
reduced `memmove` to 10.09% of whole-run samples, a 21.7% relative reduction,
while SHA-256 remained dominant at 14.83%; it also lost no samples. The
exhaustive report remained exactly `CLEAN` with zero divergences in all five
fixtures.

Twelve order-balanced CPU-pinned timing pairs ran during severe host
contention (load average 22--30, with concurrent multi-core Rust builds).
Median wall time improved from 8.695 s to 8.500 s (2.24%), while four
contention outliers made the means 9.800 s and 10.203 s respectively; six
pairs favored each binary. Process-scoped counters across six further balanced
pairs reduced mean retired instructions from 37.595B to 37.542B (0.14%).
Treat the symbolized reduction and focused capacity invariant as the stable
mechanism evidence; do not use these contended wall means as a latency
baseline.

## Long loaded-format LaTeX prefixes

Long-document profiles must use an engine work boundary, not a wall timeout.
Expansion fuel is a suitable boundary when both runs terminate with the exact
requested fuel count. Use at least two endpoints: fixed distribution and
format setup can dominate an early prefix and then decline, while command
delivery and state work should persist or grow.

Keep TeX's simulated main-memory accounting distinct from the Rust allocator.
`main_memory_projection_inner` reconstructs TeX82's live `mem` occupancy from
Umber's immutable Env, token, macro, glue, and node owners so canonical
high-water diagnostics remain correct. It does not report process RSS or
host-allocation ownership. Expanded `\write` scanning observes transient TeX
words through `observe_main_memory_dynamic_words`; clearing the retained
projection at an executor-operation boundary can therefore turn diagnostic
accounting into a repeated whole-root traversal. Profiles that find this path
hot should first count observations, full reconstructions, and successful root
updates. The positive control is repeated observation with unchanged roots;
box replacement and rollback are required negative controls.

Profile the production feature set with optimized symbols and frame pointers.
Measure a matched `profiling`-feature build separately; profiling counters are
evidence only when their retired-instruction and task-clock deltas are small
relative to run-to-run noise. A useful capture records exact work termination,
sample count and weight, call graphs, lost-sample count, pin inventories, and
the production binary hash.

## Loaded-format restoration census

A profiling-feature CLI run with `--profiling-stats` emits one
`FORMAT_RESTORE` line even when the command-fuel guard terminates the job. It
reports successful restore calls and container bytes, token words, macro
definitions, glue specs, and nodes restored; collection-level validation
passes; entries copied only to support a later pass; and explicit
restoration-owned heap buffers. These process-local counters do not enter the
format image, state, snapshots, rollback, semantic identity, cache identity,
or INITEX construction path.

The focused schema-12 Plain-format benchmark additionally counts allocator
calls and requested bytes around exactly one consuming restore, proves a
byte-identical redump, and then measures repeated fresh-Universe episodes in
which each input is validated once and consumed into its destination:

```bash
CARGO_BUILD_JOBS=1 cargo bench \
  --manifest-path benchmarks/format-restore/Cargo.toml \
  --bench decode -- --noplot
```

The work vector, rather than elapsed time, is the deterministic regression
surface. Timing and allocator observations are diagnostic mechanism evidence.
Production acceptance still uses the authenticated pinned distribution,
format, source, offline cache, exact command-fuel boundary, unchanged guards,
and a zero-loss caller/callee capture.

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

## Scalar command delivery and scanning

Issue `umber2-3v8z.27` added a monotonic command-work vector beside the
canonical fuel ledger. The vector distinguishes successful fuel charges, raw
token-frame steps, completed expanded deliveries, live meaning lookups, tokens
delivered under a non-normal scanner status, and expandable commands executed
during deferred-write expansion. The five detailed fields now exist only in
the `profiling` resolution. Default production delivery performs only the fuel
guard's decrement and exhaustion check; its stable published value reports
fuel and leaves the detailed fields zero. No runtime option or observer branch
selects accounting per token.

Profiling counters are operational evidence outside TeX state: checkpoints,
rollback, formats, semantic identity, corpus inputs, and fuel guards do not
contain or alter them. Periodic exact-vector comparisons must therefore build
both sides with the matched `profiling` feature. Production comparisons use
fuel, semantic output, and focused instruction/branch evidence instead of
treating a detailed vector as engine behavior.

Their comparison contract depends on transaction demand. When both binaries
roll back the same prefixes, all six fields must match before a CPU or
allocation comparison is accepted. Once an architecture change deliberately
commits a successful prefix before a later resource transaction, replaying
that prefix is no longer actual work. The versioned direct-prefix contract
therefore keeps fuel and raw token-frame steps exact, keeps semantic/output
identity exact, and reports the other four fields as replay-sensitive deltas
against the preserved historical vector. It never fabricates the eliminated
transitions by incrementing counters.

Because scanner-status tokens are a subset of raw frames, advancing farther
under the same raw-frame limit can increase that field while expanded
deliveries or meaning lookups decrease. Such an increase is acceptable only
when the exact raw-frame position is unchanged, the semantic gates are exact,
and a focused transaction control attributes the redistribution while proving
that direct retry adds no unrelated work. See
[`umber2-awgc.12`](writeback/umber2-awgc.12.md).

Exact profiling controls exercise 256 macro invocations and 64 macro expansions
inside deferred write text. Their respective vectors are
`(513, 512, 256, 256, 0, 0)` and `(131, 131, 1, 64, 130, 64)` in the field
order above. The tests assert the complete vectors rather than elapsed time.
The instrumented pdfTeX oracle does not currently export equivalent command
work counters, so there is no fabricated cross-engine count comparison;
existing oracle parity remains the semantic gate.

The historical endpoint captures used the authenticated pinned distribution,
format, source, and offline cache. At 6,000,000 fuel they recorded
`(6000000, 6000000, 556280, 1710443, 5383815, 433)` from 1,356 cycle samples;
at 12,000,000 fuel they recorded
`(12000000, 11999815, 1253912, 3485521, 10639579, 1136)` from 2,450 cycle
samples. Both caller/callee captures lost zero samples. Durable evidence is
under `target/umber2-3v8z.27/prod-record-fuel-6000000/` and
`target/umber2-3v8z.27/prod-record-fuel-12000000/`.

The scalar delivery loop formerly cloned the complete top token cursor on
every step, including its shared token-list and provenance owners. It now
snapshots only the cursor identity and index, then borrows the unchanged live
cursor for decoding. Raw and expanded delivery also have separately compiled
loops beneath the same typed policy entry point, removing the per-token
optional-mode branch. At the 12M endpoint, the preceding post-projection
capture attributed 12.16% flat cycles to the shared `delivery_driver_inner`.
The replacement raw driver, scalar delivery, and typed entry together account
for 9.02% (2.83%, 5.43%, and 0.76%) on the same canonical work boundary, a
25.8% relative reduction in that scalar-delivery attribution. The 6M
replacement total is 7.63% (2.74% and 4.89%; the typed entry is below 0.05%).
No meaning cache, semantic shortcut, corpus change, or guard change is part of
the optimization.

Issue `umber2-xuty` then isolated the remaining stored-token and interner
costs. Exact c167 ancestry put stored-token delivery beneath `scan_toks`, macro
definition scanning, scalar scanners, expansion, and main control. It also
assigned 1.32 profile points of interner lookup to repeated exact `par` lookup
during macro argument matching, establishing the required short-name temporal
locality before adding an accelerator.

Durable delivery now borrows and advances its already-owning token cursor
directly, with no per-word `Rc`/accounting clones or temporary advanced cursor.
The session interner retains one exact packed recent key for successful names
up to seven UTF-8 bytes; a hit compares the complete scalar key and bypasses
both hashing and `memcmp`. Five focused optimized runs reduced median stored
delivery from 164.49 to 123.27 ns/token (25.1%) and median warmed `par` lookup
from 48.69 to 17.69 ns (63.7%); both allocation scopes remained zero.

A same-host authenticated 20M capture preserved the exact work vector
`(20000000,19913119,2218327,6020965,16785710,4011)` and lost zero samples.
Against the retained c167 binary, stored-token self time fell from 4.23% to
2.67%, interner slot lookup from 3.58% to 0.62%, and interner hashing from
0.98% to 0.03%. No interner caller remained in sampled `memcmp`; its remaining
3.00% belonged to primitive resolution and distribution work. Candidate and
reference perf walls were 13.20 and 14.21 seconds, but the candidate includes
two post-c167 integration fixes, so the symbol reductions and focused controls,
not that whole-run wall delta, are the issue-local CPU evidence.

Issue `umber2-66p0.8.40.17` then fused the remaining resident command stages.
The authoritative `CommandState` transition applies one-delivery suppression
and required alignment treatment against the caller-owned destination, returns
only a copy-small ready/outer result, and skips processor observation work
entirely when no observer is present. Dense packed meaning resolution also
returns the literal catcode it already decoded, so ordinary brace
classification does not decode the spelling a second time.

Seven same-host, order-balanced one-million-token full raw-delivery pairs all
favored the candidate. Median latency fell from 65.95 to 54.70 ns/token
(17.1%); every warmed allocation scope reported zero calls and zero requested
bytes. Ten-run hardware-counter controls reduced mean cycles from 233,632,585
to 198,915,926 (14.9%), instructions from 673,426,213 to 593,375,461 (11.9%),
and branches from 110,738,895 to 101,728,871 (8.1%). Both sampled captures lost
zero samples; the candidate has no separate resident next-raw or alignment
classifier frame. This focused comparison exercises the complete stored-input
cursor, meaning resolution, resident settlement, raw next-command, and static
delivery driver rather than the cursor mutation microgate alone. The exact
assigned-base archive, binaries, paired output, counter reports, and profiles
are under `target/umber2-66p0.8.40.17/`; the coordinator retains ownership of
the final authenticated 50M production profile.

Issue `umber2-66p0.8.40.58` used the already-saved authenticated `.8.40.56`
50M capture to isolate ordinary exhausted-input retirement. Its exact default
path was `delivery_driver -> retire_input_top ->
retire_exhausted_input_with_file_warning -> pop_project`, with stack
conservation as another caller. `retire_input_top` accounted for 1.01% self and
2.61% inclusive; the disjoint selected retirement owners accounted for 2.18%
self, including 0.75 profile points directly below `delivery_driver`.

The resident `CommandState::advance_resident_command_into` transition now
pops ordinary exhausted token and macro-argument rows from its already-selected
coordinate, settles macro/replay/alignment/observation effects, and continues
from the new top. The optimized test binary retains separate symbols for the
resident transition and cold `CommandProcessor::retire_input_top`; disassembly
of the former contains direct calls only to its specialized
`InputStack::pop_resident_project` and shared `settle_input_retirement` seams,
not to `retire_input_top`, `retire_exhausted_input_with_file_warning`, or the
general `InputStack::pop_project`. The focused one-versus-4,096 gate changes the old source-boundary
probe count from `N` to zero and records four resident ordinary pops, zero
exhaustion relays/top lookups/owner validations/whole-token copies, zero whole-
frame or command copies, and zero warmed allocations. Its mixed source branch
records one explicit cold EOF; a separate control records two explicit
conservation retirements.

## Compact line-breaking routes

Issue `umber2-7asg` followed the exact ffbdb9861 20M profile's 1.691B weighted
cycles (7.82% self) in libc `memmove` with an absolute out-of-line call and byte
census. The 67,973 calls and 7,902,964 bytes were heterogeneous: format
logical-row validation, distribution selection and parsing, control-sequence
string tables, line-breaking routes, and font-store maps were the material
owners. The complete thresholded ownership table and the rounded weighted-cycle
ancestry are recorded in `docs/writeback/umber2-7asg.md`; unrelated owners
remain separate work rather than being hidden by a container substitution.

Line-breaking was the largest coherent runtime-owned value-copy family. Active
routes now hold a stable compact index into the immutable paragraph tape and
reuse its successor position and width metrics, instead of duplicating both in
every candidate. This shrank each route from 144 to 80 bytes without adding an
allocation or changing candidate ordering, passive routes, generations,
checkpoints, or replay.

On the same exact 20M interceptor boundary, out-of-line `memmove` calls fell to
62,680 (-7.79%) and bytes to 7,032,884 (-11.01%). A zero-loss candidate capture
reported approximately 1.461B weighted cycles (6.20%) self in `memmove`, down
230M cycles (-13.6%) and 1.62 profile points from the authority. Single-run
control wall time was noisy and did not improve, so the structural byte census
and sampled symbol delta, not wall time, are the performance claim.

## Structural provenance lifecycle

A profiling-feature CLI run with `--profiling-stats` emits one
`PROVENANCE_LIFECYCLE` line even when the canonical command-fuel guard stops
the run. Its fixed-width process-local counters separate ordinary origin atoms,
macro expansion frames, and origin lists: intern calls/hits/misses and actual
allocations; strong-root retains/releases; bounded weak-slot visits and
reclaims; raw-origin resolution; and list-root resolutions plus their exact
owner-search comparisons. The counters are absent from production builds and
from snapshots, formats, rollback, semantic identity, and observation data.

Focused controls cover atom/frame/list hits and misses, retain/release,
resolution, one-slot miss-side reclamation, exact hits which do not move an
unrelated sweep cursor, three inline expansion-frame children, weak-hash
collisions, rollback/retry, the 10,000-operation bounded-live plateau, and the
exact all-live negative control. Production list freezing trusts the existing
rooted-buffer alignment invariant while debug builds retain the complete
owner-membership audit.

The authenticated offline 2606.12566 census measured 208,214 frame intern
calls at 6M fuel (1,161 hits and 207,053 misses) and 406,916 at 12M (3,960 hits
and 402,956 misses). The corresponding list counts were 51,061 calls with
22,402 hits and 28,659 misses, then 100,897 with 45,250 hits and 55,647 misses.
At 12M, one-slot miss-side reclamation visited 494,853 atom slots and 55,646
list slots; list replay performed 1,980,294 root resolutions and 7,959,891
owner comparisons. These counts establish frame misses as the allocation
case, exact hits as the no-sweep control, and list resolution as distinct
borrowed work.

The historical comparison retained the exact command-work vectors recorded
above. Those vectors now belong to the matched profiling resolution rather
than the default production contract. The 6M profile collected 1,381 cycle samples with none lost;
the disjoint provenance owner fell from 512,475,473 weighted cycles (3.20%) on
the immediately preceding scalar-delivery capture to 426,847,781 (2.62%), a
16.7% reduction in attributed cycles. The 12M profile collected 2,331 samples
with none lost and fell from 1,117,490,674 cycles (3.81%) to 751,079,898
(2.68%), a 32.8% reduction. `allocate_rooted_origin_words` fell from 0.72% to
0.17% flat at 12M, and the generic SipHash leaf disappeared. Evidence is under
`target/umber2-3v8z.29/census-fuel-{6000000,12000000}/` and
`target/umber2-3v8z.29/prod-record-fuel-{6000000,12000000}/`.
