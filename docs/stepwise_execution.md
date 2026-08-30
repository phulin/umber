# Stepwise TeX execution and resource suspension

This document defines the implementation contract for owned, stepwise
`MainControl` execution. It is the
engine-side layer beneath the host-neutral resource protocol in
[`wasm_resource_acquisition.md`](wasm_resource_acquisition.md) and the accepted
revision transaction in
[`persistent_compile_sessions.md`](persistent_compile_sessions.md). The named
incremental checkpoint schedule remains the one defined by
[`incremental_v1.md`](incremental_v1.md); executor steps are private rollback
points and do not add checkpoint kinds.

The central rule is:

> Execution suspends by rolling one bounded candidate step back to its stable
> entry state and replaying that step after resource registration. It never
> preserves an arbitrary Rust call stack.

This deliberately trades some repeated work at a miss for simple ownership,
bounded retained state, deterministic retries, and one implementation on
native and `wasm32-unknown-unknown` targets.

## Implementation status

`tex-exec` exposes the owned canonical control state and the explicit
`JobStart`, `MainControl`, `FinishEnd`, and `Finalize` lifecycle. Retained
sessions drive this state machine directly. Main control yields after at most
256 fully dispatched tokens or fixed 256-token text spans, and yields
immediately after a named paragraph or shipout boundary. TeX group entry and
exit are no longer yield points: ordinary episodes cross them while the save
stack preserves their restoration records. Named checkpoints are staged and
delivered only after that bounded operation chunk returns successfully.
The command core retains its bounded traced-token scratch pool outside
`MainControl`, semantic state, direct-operation cursors, and named checkpoints.
Font, image, and read-recorder host capabilities remain borrow-scoped.
Typed prepared continuations own resource retry, while validated
`CommandSummary` is the only durable named-checkpoint continuation.

`MainControl` preflights each settled command before operand scanning.
Successful ordinary commands run directly after advancing the environment
write epoch and commit the node-operation watermark; they own no aggregate
savepoint.
Commands that may expand or settle a prefix/reswitch command into a resource
request, publish PDF/effect/output state, apply ErrorStop recovery, or allocate
in a private revision retain the one-operation aggregate adapter pending their
narrow-mark migration. The closing brace of an active box is promoted to that
adapter when packaging can run the page or explicit-shipout pipeline; ordinary
groups nested inside the box remain on the direct path.
ErrorStop prompting returns a typed interaction outcome. Command-side reports
apply it immediately; executor-side reports carry one outcome in the existing
operation-local diagnostic handoff to the canonical processor seam. Ordinary
raw and expanded delivery never inspect world error state.
`CanonicalStepRunner` and its `OutputLedger` are the shared native/incremental
publication protocol above that transition. A typed resource need restores the matching `Universe`,
command-state, mode-nest, execution, statistics, checkpoint-publisher, prepared
page, diagnostic/effect/artifact, and lifecycle roots before returning
`AwaitingResources`; the suspension serial remains monotonic and replay uses
the original logical resolution index. Named checkpoints and external read
observations are staged and delivered only after the candidate commits.

`tex-incr::RevisionCandidate` and `umber::EngineSession` now provide
the host-session retention layer. A candidate begins from an accepted
checkpoint's `CommandSummary` and aggregate execution roots, or from
`JobStart` when no checkpoint is eligible. It owns that canonical control,
input stack, mutable `Universe`, speculative checkpoint sink, editor setup, and
private workspace generation across resource batches. Each drive installs
resolvers over a fresh immutable VFS snapshot and therefore replays only the
rolled-back executor step. The host tracks response progress separately and
rejects a retry that binds no newly awaited positive or
authoritative-negative response.

After a candidate that explicitly requests the `Pdf` output capability reaches terminal engine execution, the incremental
owner may borrow that completed candidate's `Universe` only for downstream
immutable resource finalization. VF/local-TFM/map/encoding/program discovery
can therefore suspend the still-unaccepted candidate and resume against a new
VFS generation without publishing its revision. Incomplete candidates never
expose live state, and packet lowering remains after the acceptance barrier.
The engine name and `\pdfoutput` state do not activate this discovery; HTML-
and DVI-only pdfTeX-compatible sessions skip it.

Canonical command fuel has a monotonic per-run ledger outside the step
savepoint. A resource rollback restores semantic command state without
refunding work. The retired expansion path retains its separate
`SessionLimits::engine_fuel` compatibility guard, but that guard is not
authoritative for canonical delivery or scanner termination.
`ExecutionTelemetry` reports one cold start
per owned run, advance calls, suspensions, local step retries, replayed
delivered tokens and dispatches, cumulative expansion fuel, and engine time.
The virtual compile layer adds host resource-wait time without changing the
engine's deterministic state.

## Public shape and ownership

`tex-exec` exposes one borrow-scoped runner over the owned canonical control and
revision state:

```rust
pub struct CanonicalStepRunner<'a> { /* borrowed control, state, and ledger */ }

pub enum CanonicalStepResult {
    Progress(MainControlStep),
    ResourceNeed(ResourceNeed),
    Committed(MainControlStep),
    Completed(MainControlStep),
    Failed(CanonicalStepFailure),
}
```

`ExecutionStep` is the next stable operation recorded in the run, not a caller
command. `CanonicalStepRunner::step` executes at most that operation and
returns its result. Its call borrows checkpoint delivery and cancellation;
resource resolvers remain host-owned. No resolver, sink, recorder, JavaScript
value, future, filesystem handle, or other host capability is retained by the
runner or ledger.

`ExecutionProgress` reports the committed next step and zero or more detached
named checkpoints. `ResourceSuspension` owns a sorted, deduplicated request
batch, the blocked `ResourceSite`, and a monotonic suspension serial. It does
not expose an engine snapshot. `Complete` owns the finalized statistics and
can be returned idempotently by higher session layers; those layers reject
further driving after a terminal state.

The existing one-shot `run*` methods become adapters which construct a run and
call `step` until terminal. They must use the same typed service adapters and
must not retain a synchronous-only path.

## Complete live-state inventory

Everything that currently survives only because `run_session` and its callees
remain on the stack lives in `MainControl`, `Universe`, or the retained session:

| Owned component      | Required contents                                                                                                                                                                                                                                                        |
| -------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `CommandState`       | All source and token-list frames, source/replay/condition allocators, alignment interception, macro invocation provenance, transient command replay, cursors, command profile, and persistent expansion state.                                                           |
| `Universe`           | Environment and group journal, content/node stores, page builder, PDF ledger, input summary, virtual streams and effects, committed artifact sequence, job clock, random state, and every rollback/hash root described by `core_state.md`.                               |
| `ModeNest`           | Every mode level, list root and scalar, pending horizontal characters, and alignment/math/box submode state.                                                                                                                                                             |
| `ExecutionStats`     | Delivered and fully dispatched token counts, text-span counts, dump flag, committed prepared DVI page plans, shipped-artifact suffix, and final DVI plans. Candidate increments are not published early.                                                                 |
| checkpoint publisher | Whether `JobStart` has been emitted, the cached mode projection, boundary occurrence/schedule state, staged `EngineCheckpoint`s, and stop-after-boundary policy. The caller-owned `CheckpointSink` is not retained.                                                      |
| run artifact state   | The artifact/effect prefixes at run entry, prepared-page queues keyed by artifact identity, and the committed prefix belonging to the current run.                                                                                                                       |
| command runtime      | The discardable two-slot traced-token scratch pool retained across successful calls. It enters neither direct-operation cursors nor named checkpoints and is cleared at its owning command boundary.                                                                     |
| lifecycle            | The next `ExecutionStep`, whether `\end` or `\dump` was seen, end/output cleanup progress, terminal result, and cancellation latch. Recursive output, alignment, math, and scanner frames never appear here: a step either unwinds them successfully or rolls them back. |
| output state         | Pending page fire-up already represented in `Universe`, prepared DVI pages in stats, recoverable/terminal diagnostics in the virtual `World`, and generated output/effect prefixes in the private build stage.                                                           |
| accounting           | Committed execution statistics, cumulative fuel, hard fuel limit, advance count, suspension serial, and optional failure-injection sequence.                                                                                                                             |

Main control additionally retains one failure-only causal diagnostic context.
The first committed recoverable error freezes only its stable family plus the
total input/group depths and the innermost eight content-free frame/group
descriptors. The field participates in aggregate step rollback, so a resource
retry cannot publish evidence from an attempt that did not commit. A later
fatal reuses that first context instead of replacing it with terminal cleanup
state. Runs without an error allocate no diagnostic context and perform only
the existing scalar error-count observation at the operation boundary.

`MainControl` owns `CommandState`, mode/execution roots, fuel ledger,
and explicit host capabilities. Each operation constructs borrow-scoped
`CommandProcessor` and `CommandHostContext` values; the processor directly
borrows the ledger's singular `CommandFuel` and has no owned-fuel form. Neither
borrowed facade can outlive the operation or enter a snapshot.

`CommandSummary` is deliberately not the private per-step savepoint: summary
publication requires a durable quiescent boundary. `CommandStateSnapshot`
instead captures cursor, transient replay, expansion, scanner, and alignment
state over already owned backing without calling a host. Named checkpoint
publication validates quiescence and stores the resulting `CommandSummary` as
its only command/input continuation.

## Lifecycle state machine

```text
Created(JobStart)
    | committed step
    v
Ready(MainControl) -- ordinary progress --> Ready(MainControl)
    | resource need                         | \end or \dump
    v                                       v
AwaitingResources --------------------> Ready(same step)   Ready(FinishEnd)
    | cancellation / hard error                  |             |
    v                                            |             v
Cancelled or Failed <----------------------------+       Ready(Finalize)
                                                               |
                                                               v
                                                            Complete
```

`EndOfInput` goes directly from `MainControl` to `Finalize`; explicit `\end`
goes through `FinishEnd`. `\dump` also goes through the explicit end path, then
`Finalize` clears the input summary and resets job-local page state exactly as
the current loop does.

`AwaitingResources` always names the same next `ExecutionStep` that was rolled
back. Resource registration changes only the workspace's immutable
resource generation. A complete or partial response batch moves the host
session back to `Ready`; replay may suspend again with the remaining or newly
discovered requests. Retrying without a newly bound positive or authoritative
negative answer is the existing typed no-progress failure.

Named checkpoint stop requests are honored only after the candidate step and
the named checkpoint have committed. They return ordinary `Progress`; they do
not create a terminal executor lifecycle or a resource suspension. Incremental
code may later continue the same run or restore a published checkpoint through
its existing aggregate operation.

## Atomic executor steps

The four step kinds have these exact commit boundaries:

1. `JobStart` synchronizes source ids and job clock, queues `\everyjob`, and
   stages the single `JobStart` checkpoint. It commits all of those together.
2. `MainControl` begins immediately before recoverable-diagnostic draining and
   ordinary expanded delivery. It processes at most 256 fully expanded
   delivered tokens or fixed-size text-span chunks, including dispatch and all
   pending output work caused by each operation.
   It commits early after a named paragraph or shipout boundary. Group changes
   are ordinary journal/save-stack transitions within the same bounded
   episode. End-of-input flushing is also one main-control step.
3. `FinishEnd` performs the current `finish_end`, including final paragraph,
   page-builder, output-routine, recursive dispatch, and shipout work. If this
   proves too large under focused measurements it may be decomposed into
   explicit owned end phases, but only at states which own all continuation
   inputs; arbitrary stack suspension is forbidden.
4. `Finalize` publishes the final input summary, applies `\dump` cleanup,
   selects the run's artifact suffix, matches prepared DVI plans by hash and
   occurrence, and compiles any missing plans. The complete statistics become
   visible only when this step commits.

A main-control step includes `get_x_token`, scanner work, the complete stomach
dispatch, `drain_pending_output`, boundary observation, and staged checkpoint
capture. Consequently a miss in a scanner or recursive output routine replays
the token's expansion too. Resource response keys, virtualized clock/random
values, and resolver selection are stable, so replay is semantically
identical. The fixed text-span chunk size is part of the executor version and
prevents one large span from becoming an unbounded atomic operation.

Pure line breaking, packing, page cost calculation, and artifact serialization
remain effect-free. Their inputs are owned values or roots. Expensive loops
must have existing structural limits or gain explicit hard input/work limits;
they are not made interruptible by retaining an iterator frame.

## Direct operation and rollback protocol

Production execution has no aggregate step savepoint. Ordinary, resource,
effect, PDF/page, ErrorStop, observed, tracked, checkpoint-crossing,
active-alignment, diagnostic-expansion, and output-capable box-closing commands
settle delivery and scanning before direct semantic apply. Typed resource
continuations retain completed operands without restoring command input; an
observed continuation also moves its unpublished evidence and opaque
delivery-order cursor rather than cloning or reconstructing them.

`DirectOperationMark` is fixed-size and non-restoring. It owns the current
environment-journal cursor and, for an incremental candidate, disposable
private-allocation watermarks. It registers no aggregate rollback root and
does not construct or advance semantic state identity. A successful operation
closes the private mark and may establish a new level-zero journal baseline
only when no named checkpoint or fork prefix retains the old one. Open groups,
delivered checkpoints, and inherited fork authority preserve their exact
restoration records. A failed operation drops unpublished scratch allocations;
canonical partial semantic state and an already prepared resource
continuation remain authoritative.

The episode returns after a world effect so the host can publish same-run
output before a later command probes it and can enforce the pending-effect
budget at the first exceeding operation. Cumulative accounting and the
cancellation latch are monotonic operational state. A candidate accumulates
named checkpoints, read-recorder observations, and diagnostic/effect output in
private state. The completion protocol is:

1. validate the candidate's mode/group invariants, prepared artifact mapping,
   limits, and next phase;
2. commit the semantic roots and generated-stage suffix;
3. increment committed `ExecutionStats` and advance `next_step`;
4. close the direct operation; then
5. deliver detached checkpoints and committed read observations to call-local
   sinks.

Sink delivery cannot fail semantically. A host that cannot retain a checkpoint
must decline it through `wants_checkpoint` before capture; it cannot make an
already committed TeX step fail. A checkpoint sink's stop decision is sampled
for the next return only.

On a typed resource need, the run retains the prepared request and enters
`AwaitingResources`. No restoration calls host policy. A diagnostic-oriented
runner returns a captured TeX82 §93 fatal with its source site after closing
the failed direct operation.

Command-side suspension is a structural ownership chain, not a retry-order
queue. The pending operation, preflight, diagnostic, or alignment phase owns
one move-only root continuation key. Each nested scanner or expansion frame
owns its exact child key and return destination in the current generation's
ABA-tagged reusable scratch lane. Resume consumes the caller edge directly;
abort closes children before parents. A scanner configuration mismatch is an
invariant failure, never permission to search, repair, rehome, or skip a frame.
The retained complete-job owner instead uses the runner's fatal-completing
entry: §81's `jump_out` latches semantic completion at §1332's `end_of_TEX`,
then the owner performs §1333 cleanup. Neither path retries the unavailable
input. Host-protocol and execution-budget failures enter `Failed`.
Cancellation is checked before mutation. A Rust panic is not a supported
suspension and does not promise recovery.

The existing lifetime-bound `ExecutionTransaction` remains useful inside
recursive submodes but is not an aggregate outer savepoint.

## Resource protocol and request sites

Resolver operations return a typed internal result:

```text
ResourceLookup<T> = Available(T) | Unavailable | NeedResource(ResourceNeed)
```

`Need` is carried through `ExpandError`/`ExecError` without conversion to a
diagnostic string. `Unavailable` is an authoritative registered answer and
continues through ordinary TeX missing-input, false-probe, or missing-font
semantics. Typed extraction recursively traverses captured errors and every
integer, dimension, glue, and general-text scanner wrapper, including scanner
wrappers nested inside other scanners. The complete public request key, not a
resolver URL or the numeric request index, is its identity.

The resource classes are:

- `InputFile`: required `\input` and stream reads, plus blocking existence,
  size, modification-date, and content probes;
- `Font`: classic TFM metrics and/or the selected OpenType program and instance
  required before font-dependent shaping or layout; and
- `ExternalImage`: the exact external image object and parse selection,
  including page, page box, and resolution.

Requests additionally carry `Required` or `Probe`. A probe is optional only in
the TeX sense that authoritative absence has normal behavior; it is blocking
until the host supplies bytes or absence. Prefetch hints are host/session
optimizations and never suspend the canonical step runner.

The `ResourceSite` recorded for diagnostics and failure injection is one of
`Expansion`, `MainControl`, `ParagraphFinish`, `LineBuild`, `PageBuild`,
`Shipout`, `FontLoad`, `ExternalImageParse`, or `EndFinalization`. Site does not
change request identity or atomicity:

| Site                                  | Contract on `Need`                                                                                                                                                                                                                               |
| ------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| expansion                             | Roll back token acquisition, macro/scanner frames, input cursors, resolution index, staged read-recorder observations, recoverable diagnostics, and all expansion fuel state except cumulative run fuel.                                         |
| main-control dispatch                 | Roll back the delivered token, assignment/group mutations, mode changes, virtual effects, and candidate statistics; replay starts before expansion of that token.                                                                                |
| paragraph finishing and line building | Roll back the entire paragraph-ending dispatch, line nodes, contribution/page-list changes, and any output it triggered. Pure line breaking itself must not call a host; resource-dependent font/shaping inputs are resolved before entering it. |
| page building                         | Roll back contribution consumption, page totals, insertion splitting, fire-up state, and candidate output work. Pure page-cost calculation has no resolver.                                                                                      |
| shipout                               | Roll back box removal, deferred-write expansion, stream state, image/font selection, detached effects, artifact bytes, prepared DVI plans, and page-node release. `ShipoutComplete` is staged only after the outermost shipout commits.          |
| font loading                          | Roll back request-index advancement and every partially scanned `\font` assignment. Parsing supplied bytes occurs during resource registration; selection is recorded in `World` only on replay.                                                 |
| external image parsing                | Registration validates and pins the external object first. A need during `\pdfximage` or shipout rolls back the whole containing step; no partial PDF object, dimension, or image ledger entry survives.                                         |
| end-of-job finalization               | Roll back final paragraph/page cleanup, output-routine work, shipouts, final summaries, and artifact-plan assembly. Replay re-enters `FinishEnd` or `Finalize`, never an internal output frame.                                                  |

A suspension batch contains every request synchronously emitted by the blocked
lookup operation before its first unavailable dependency, sorted and
deduplicated by typed key. Default-extension candidates, paired font
containers, and image dependencies may therefore batch. Execution does not
speculate past a missing required value merely to discover unrelated future
requests; manifest closures remain prefetch hints.

## Atomicity invariants

The implementation and tests must preserve all of these invariants:

1. `AwaitingResources` is observationally equal to the stable entry of its
   blocked step, excluding cumulative accounting and the detached request.
2. Replaying with the same registered resources produces the same request or
   committed next state independent of native/WASM host, response order, and
   response batch partitioning.
3. No diagnostic, stream write, generated byte, artifact ordering entry, DVI
   plan, checkpoint, or read-recorder callback from
   a rolled-back candidate is visible.
4. Previously committed executor steps remain private but intact across a
   later suspension. Rejecting the whole compile discards the enclosing VFS
   build/revision transaction; accepting it publishes all committed steps at
   once.
5. A resource response never mutates TeX semantic state directly. It extends
   the immutable workspace generation; the replayed TeX operation observes
   and records the selection at its original point in execution order.
6. A named checkpoint is captured only at the existing `JobStart`, eligible
   `OuterParagraphEnd`, and eligible outermost `ShipoutComplete` schedule. Step
   savepoints are private, unhashed, unretained after commit, and never offered
   for incremental restart. Under the temporary conservative source policy,
   paragraph and shipout eligibility is frozen when the forming operation sees
   the active external file frame: only the registered root main file is
   retained. Later input retirement or resource resumption cannot change that
   scalar decision. General source roles remain deferred to Bead
   `umber2-66p0.11`.
7. Resource suspension cannot bypass a TeX group, shipout transaction, output
   routine, or hard limit.
8. Successful stepwise output, statistics, checkpoint schedule, effects,
   artifacts, and generated files are byte-for-byte and order-for-order equal
   to the one-shot adapter with the same resources.

## Counters, fuel, cancellation, and hard limits

Counters fall into two classes.

Rollback-coupled counters include `ExecutionStats`, input source/replay and
condition allocators, `Universe` allocation and PDF cursors, effect/artifact
positions, prepared-page occurrence counts,
checkpoint occurrence state, and the expansion `resolution_index`. Restoring
the savepoint restores these exactly. The replayed lookup therefore receives
the same resolution index; wraparound becomes a typed hard-limit failure
rather than the current wrapping behavior.

Monotonic counters are `advance_calls`, committed executor steps,
`suspension_serial`, response-progress generation, failure-injection sequence,
and `cumulative_fuel`. They are outside
the savepoint, never decrease on retry, and are telemetry or abuse-control
state rather than TeX semantics. Request identity never depends solely on one
of these counters. Committed steps and cumulative fuel affect future budget
decisions, so named checkpoints retain both; scanner and alignment watchdog
state remains live-run-only and never becomes a continuation.

Fuel is charged before each expansion loop action, delivered-token dispatch,
text-span token, builder unit, shipped node/event, and finalization unit. Work
performed by a candidate that later rolls back remains charged. This prevents
a document from resetting its budget by causing
resource misses. Canonical execution charges the shared
`tex_command::CommandFuel` ledger before raw delivery. Crossing a hard limit detaches
a typed error, rolls back the current step, and terminally fails that candidate.
It is not `AwaitingResources` and cannot be retried by increasing a limit.

Diagnostic provenance is append-only work owned by the candidate `Universe`,
not a semantic progress measure. A scanner that fails to consume its canonical
input can therefore exhaust the provenance arena and raise process RSS even
when the live node graph and physical source cursor are nearly stationary.
The integer scanner follows TeX.web §429 here: an internal `\dimexpr` requested
by `\number` is fully scanned through its terminating `\relax`, then lowered to
raw scaled points. Leaving that expression on input caused hyperref's UTF-8
PDF-string macros to replay the same byte indefinitely; the focused regression
repeats the nested-expression shape 4,096 times and bounds both cumulative fuel
and provenance retention. Fuel and RSS guards remain terminal backstops rather
than substitutes for progress-preserving scanner semantics.

`step` itself is the cooperative scheduling boundary. It never returns halfway
through a scanner or pure algorithm. Native callers may loop; WASM callers
return to JavaScript after each `Progress` and may schedule the next call in a
microtask or worker turn. Fixed text chunks plus node/input/work hard limits
bound a step. Measurements may justify finer explicit owned phases, but a
platform-specific continuation or `async` engine fork is forbidden.

Cancellation is a monotonic latch checked before a step and at designated
bounded polls inside expansion, paragraph/page loops, and shipout traversal.
Observation inside a candidate unwinds with a private cancellation marker,
rolls the whole step back, and terminally returns `Cancelled`. Cancellation
emits no TeX diagnostic and publishes no staged checkpoint or output. A
resource response received after cancellation is not transferred into the
run; shared host caches may retain verified immutable bytes. At the persistent
session layer, cancelling a pending editor revision drops its canonical control
and private VFS/revision transaction while preserving the last accepted
revision and immutable resource bindings.

Production sessions default to 10,000,000 committed steps, 100,000 live input
frames, 256 MiB of environment journal, 1,000,000 pending effects, and
100,000,000 command-fuel units. Engine sessions accept only
`1..=100,000,000,000`; zero and larger values are typed configuration errors.
`SessionLimits` configures the legacy ceilings uniformly for native and WASM
sessions. Native CLI runs expose the expansion and committed-step guards as
the independent `--expansion-fuel` and `--execution-steps` flags, with
`UMBER_ENGINE_FUEL` and `UMBER_ENGINE_STEPS` retained as compatibility
fallbacks. They additionally accept `UMBER_INPUT_FRAMES`,
`UMBER_JOURNAL_BYTES`, and `UMBER_EFFECTS`.

Node, input-depth, recursion, output, generated-file, resource, and decoded
font/image limits remain hard terminal errors. A limit reached during a step
uses the same rollback protocol. Candidate bytes are counted before allocation
or publication; cumulative fuel and committed-step accounting are not refunded
by rollback.

## Native, WASM, and build composition

Both hosts drive the same Rust `CanonicalStepRunner` and typed result values. A native
adapter may satisfy a request immediately and call `step` again on the same
thread. WASM serializes `ResourceSuspension`, returns to JavaScript for
asynchronous acquisition, registers validated responses through the shared
session, and calls the same `step` again. Rust never blocks on a future, derives
a URL, or retains a JavaScript resolver.

The run lives inside one private `umber-vfs` build stage. Committed executor
steps may append virtual generated effects that are visible to later steps of
that same candidate build. A step savepoint rolls back only its candidate
suffix; a resource suspension retains earlier committed step prefixes. A
terminal run failure discards the complete stage/build, while successful
final output validation accepts the VFS build, incremental revision,
diagnostics, artifacts, and returned output together as specified by
`persistent_compile_sessions.md`.

Provisioned immutable resources may survive a failed or cancelled candidate
in the session cache. They are not generated output and do not imply revision
acceptance. Native direct output remains deferred until accepted finalization;
WASM memory output remains detached. Neither platform exposes host effects
that would need to be undone.

## Command-state cutover

The cutover is complete: canonical candidates retain `CommandState`, resource
suspension moves typed scanner or expansion continuations, and named
checkpoints store a validated `CommandSummary`. Isolated shipout text expansion
may snapshot and restore only its nested synthetic input transaction; it is not
a main-control retry authority. `tex-incr` and Umber resume through the durable
checkpoint plus explicit executor/runtime roots. There is no lexer/expander
retry snapshot, reconstruction adapter, or whole-revision compatibility
restart.

## Focused tests and failure injection

Unit tests inject `Need`, cancellation, and hard failure at deterministic
`(ExecutionStep, ResourceSite, operation_ordinal)` points. Every injection
compares a before/after aggregate projection containing input summary and
transient replay, mode summary, `Universe` state hash, group depth, page state,
effect/artifact positions, stats, expansion index, pending fire-up, prepared
pages, and staged generated-output prefix.

Required focused cases are:

- input need during macro expansion, delimited argument scanning, `\input`,
  `\openin`, and file metadata/content probes;
- font need after a partially scanned `\font` assignment and immediately
  before paragraph shaping/line breaking;
- image need during `\pdfximage` parsing and during deferred shipout traversal;
- suspension while ending a paragraph, splitting insertions, firing an output
  routine, recursively dispatching output tokens, and forced end-of-job
  shipout;
- suspension after candidate terminal/log text, immediate/deferred stream
  writes, generated file writes, an artifact, and a prepared DVI page have each
  been produced;
- identical resource response orders and partial batches yielding identical
  request keys, expansion indices, checkpoint schedules, artifacts, generated
  files, diagnostics, statistics, and final bytes;
- cancellation before a step and at every bounded poll, including while
  awaiting resources, with the accepted revision and retained-byte accounting
  unchanged;
- cumulative fuel charged across repeated suspensions, per-expansion fuel
  exhaustion, checked resolution-index exhaustion, and node/output/resource
  hard limits, all without leaked candidate effects;
- a sink requesting stop at each named boundary, proving delivery occurs once
  after commit and resume neither duplicates `JobStart` nor loses the next
  step; and
- one-shot versus stepwise cold, formatted, incremental, native, direct-WASM,
  and worker-WASM parity over multi-resource documents.

Failure hooks must exist immediately before and after input consumption,
resolver calls, semantic mutations, paragraph publication, page fire-up,
effect append, artifact detach/commit, node release, checkpoint capture,
generated-stage append, and final stats/DVI assembly. Test-only hooks report
typed internal stops; production builds retain no dynamic callback or branch
beyond the ordinary cancellation/fuel polls.

The rollout is complete only when no live execution local required for replay
remains solely on `run_session`'s stack, no missing-resource path requires a
whole-revision restart, and native/WASM outputs remain cold-equivalent under
failure injection at every listed boundary.
