# Stepwise TeX execution and resource suspension

This document defines the implementation contract for replacing
`tex_exec::Executor::run_session` with an owned, stepwise executor. It is the
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

`tex-exec` now exposes the owned `ExecutionRun` and `ExecutionState`, the
call-local `ExecutionServices`, and the explicit `JobStart`, `MainControl`,
`FinishEnd`, and `Finalize` lifecycle. The compatibility `run*` and `resume*`
entry points drive this same state machine. Main control yields after at most
256 fully dispatched tokens, paragraph-reuse operations, or fixed 256-token
text spans, and yields immediately after a named paragraph or shipout boundary;
it also yields whenever TeX's execution-group depth changes so the environment
snapshot remains a valid rollback root. Named checkpoints are staged and
delivered only after that bounded operation chunk returns successfully.
Expansion state is detached from input,
font, image, and read-recorder capabilities between calls.

`tex-lex` now provides the opaque `InputStackSnapshot` prerequisite. Capture
retains complete owned source cursors and private input machinery, while
rollback is infallible and resolver-free; `InputSummary` remains the durable
publication format.

`ExecutionRun` now wraps every candidate operation in a private aggregate
`StepSavepoint`. A typed resource need restores the matching `Universe`, opaque
input-stack, mode-nest, execution, statistics, checkpoint-publisher, prepared
page, diagnostic/effect/artifact, and lifecycle roots before returning
`AwaitingResources`; the suspension serial remains monotonic and replay uses
the original logical resolution index. Named checkpoints and external read
observations are staged and delivered only after the candidate commits.

`tex-incr::RevisionCandidate` and `umber::VirtualCompileSession` now provide
the host-session retention layer. A candidate owns its `ExecutionRun`, input
stack, mutable `Universe`, speculative checkpoint sink, paragraph-memo
generation, editor setup, and private VFS provisioner generation across
resource batches. Each drive installs resolvers over a fresh immutable VFS
snapshot and therefore replays only the rolled-back executor step. The host
tracks response progress separately and rejects a retry that binds no newly
awaited positive or authoritative-negative response.

After a candidate that explicitly requests the `Pdf` output capability reaches terminal engine execution, the incremental
owner may borrow that completed candidate's `Universe` only for downstream
immutable resource finalization. VF/local-TFM/map/encoding/program discovery
can therefore suspend the still-unaccepted candidate and resume against a new
VFS generation without publishing its revision. Incomplete candidates never
expose live state, and packet lowering remains after the acceptance barrier.
The engine name and `\pdfoutput` state do not activate this discovery; HTML-
and DVI-only pdfTeX-compatible sessions skip it.

Expansion fuel also has a monotonic per-revision counter outside the step
savepoint. A resource rollback restores semantic expansion state without
refunding work, and `SessionLimits::engine_fuel` terminally rejects a candidate
that crosses its finite ceiling. `ExecutionTelemetry` reports one cold start
per owned run, advance calls, suspensions, local step retries, replayed
delivered tokens and dispatches, cumulative expansion fuel, and engine time.
The virtual compile layer adds host resource-wait time without changing the
engine's deterministic state.

## Public shape and ownership

`tex-exec` owns the run as `ExecutionRun`. The intended API shape is:

```rust
pub struct ExecutionRun { /* private owned state */ }

pub enum ExecutionStep {
    JobStart,
    MainControl,
    FinishEnd,
    Finalize,
}

pub enum ExecutionStepResult {
    Progress(ExecutionProgress),
    AwaitingResources(ResourceSuspension),
    Complete(ExecutionStats),
    Failed(ExecError),
    Cancelled,
}
```

`ExecutionStep` is the next stable operation recorded in the run, not a caller
command. `ExecutionRun::step(&mut ExecutionServices, &Cancellation)` executes
at most that operation and returns its result. `ExecutionServices<'_>` borrows
input, font, image, read-recording, and checkpoint-delivery adapters for one
call. No resolver, sink, recorder, JavaScript value, future, filesystem handle,
or other host capability is retained in `ExecutionRun`.

`ExecutionProgress` reports the committed next step and zero or more detached
named checkpoints. `ResourceSuspension` owns a sorted, deduplicated request
batch, the blocked `ResourceSite`, and a monotonic suspension serial. It does
not expose an engine snapshot. `Complete` owns the finalized statistics and
can be returned idempotently by higher session layers; `ExecutionRun::step`
itself rejects further driving after a terminal state.

The existing one-shot `run*` methods become adapters which construct a run and
call `step` until terminal. They must use the same typed service adapters and
must not retain a synchronous-only path.

## Complete live-state inventory

Everything that currently survives only because `run_session` and its callees
remain on the stack moves into `ExecutionRun` or an owned component reached by
it:

| Owned component      | Required contents                                                                                                                                                                                                                                                               |
| -------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `InputStack`         | All source and token-list frames, source/replay/condition allocators, alignment interception, macro invocation provenance, transient token replay, cursors, lexer configuration, caches, and expansion statistics.                                                              |
| `Universe`           | Environment and group journal, content/node stores, page builder, PDF ledger, input summary, virtual streams and effects, committed artifact sequence, job clock, random state, and every rollback/hash root described by `core_state.md`.                                      |
| `ModeNest`           | Every mode level, list root and scalar, pending horizontal characters, alignment/math/box submode state, and paragraph continuation state.                                                                                                                                      |
| `ExecutionStats`     | Delivered and fully dispatched token counts, text-span counts, dump flag, committed prepared DVI page plans, shipped-artifact suffix, and final DVI plans. Candidate increments are not published early.                                                                        |
| checkpoint publisher | Whether `JobStart` has been emitted, the cached mode projection, boundary occurrence/schedule state, staged `EngineCheckpoint`s, and stop-after-boundary policy. The caller-owned `CheckpointSink` is not retained.                                                             |
| run artifact state   | The artifact/effect prefixes at run entry, prepared-page queues keyed by artifact identity, and the committed prefix belonging to the current run.                                                                                                                              |
| expansion state      | `EngineStateSnapshot`, job name and clock, resolution index, meaning-site cache, macro replay site, nesting depths, recoverable diagnostics, expansion-fuel scope, and all paragraph read/dependency recording state. Resolver and recorder references are call-local services. |
| paragraph memo state | Pending continuation, barrier and disabled-for-run flags, cold recording including effect start and input/span anchors, local boxes, inline-math reads, dependency cache, and the paired `Universe` pure-paragraph recording root.                                              |
| lifecycle            | The next `ExecutionStep`, whether `\end` or `\dump` was seen, end/output cleanup progress, terminal result, and cancellation latch. Recursive output, alignment, math, and scanner frames never appear here: a step either unwinds them successfully or rolls them back.        |
| output state         | Pending page fire-up already represented in `Universe`, prepared DVI pages in stats, recoverable/terminal diagnostics in the virtual `World`, and generated output/effect prefixes in the private build stage.                                                                  |
| accounting           | Committed execution statistics, cumulative fuel, hard fuel limit, advance count, suspension serial, and optional failure-injection sequence.                                                                                                                                    |

The present borrowing `ExecutionContext<'a>` is split into an owned
`ExecutionState` containing the expansion and paragraph fields above and the
call-local `ExecutionServices<'a>`. The run owns the job name instead of
borrowing `&str`.

`InputSummary` is not sufficient for a per-step savepoint: reconstructing it
can reopen sources and therefore can itself request a resource. `tex-lex` must
provide an opaque `InputStackSnapshot` plus infallible rollback over already
owned backing records. It captures cursor and transient replay state without
calling a resolver. This may initially be an owned clone of the live stack;
the representation may later become copy-on-write. Publication summaries
remain the durable named-checkpoint representation.

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
back. Resource registration changes only the provisioner's immutable
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
2. `MainControl` begins immediately before paragraph-reuse probing and
   recoverable-diagnostic draining. It processes at most 256 fully expanded
   delivered tokens, paragraph reuses, or fixed-size text-span chunks,
   including dispatch and all pending output work caused by each operation.
   It commits early after a named paragraph or shipout boundary or any
   execution-group depth change. End-of-input flushing is also one main-control
   step.
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

## Savepoint and rollback protocol

Immediately before a candidate step, the run captures `StepSavepoint`:

```text
StepSavepoint {
    universe: Snapshot,
    input: InputStackSnapshot,
    modes: ModeNest rollback root,
    execution: ExecutionState rollback root,
    stats: ExecutionStats rollback root,
    checkpoint_publisher: publisher rollback root,
    artifact/effect/prepared-page prefixes,
    generated-stage savepoint,
    next_step and end flags,
}
```

The savepoint excludes cumulative accounting and the cancellation latch. A
candidate accumulates named checkpoints, read-recorder observations, and
diagnostic/effect output in private state. The completion protocol is:

1. validate the candidate's mode/group invariants, prepared artifact mapping,
   limits, and next phase;
2. commit the semantic roots and generated-stage suffix;
3. increment committed `ExecutionStats` and advance `next_step`;
4. release the savepoint; then
5. deliver detached checkpoints and committed read observations to call-local
   sinks.

Sink delivery cannot fail semantically. A host that cannot retain a checkpoint
must decline it through `wants_checkpoint` before capture; it cannot make an
already committed TeX step fail. A checkpoint sink's stop decision is sampled
for the next return only.

On a typed resource need, the run first detaches the request payload, then
restores every field in `StepSavepoint` and enters `AwaitingResources`. No
restoration calls host policy. Terminal TeX errors preserve the live failure
state, matching the one-shot interpreter contract; their staged checkpoints
and read observations are discarded, and the run enters `Failed`. Cancellation
is checked before mutation. A Rust panic is not a supported suspension and does
not promise recovery.

On native hosts, observational telemetry times savepoint capture and rollback
separately from the remaining engine step body. This distinguishes local retry
cost from host resource resolution without changing the savepoint boundary or
allowing a retry to skip restoration.

The `Universe` snapshot and input snapshot must be taken and restored as one
aggregate operation. No caller may roll back input, modes, paragraph recording,
execution state, or `Universe` independently. A resource lookup may suspend
after the blocked operation has entered nested TeX groups; the environment's
lineage check admits rollback through those still-live descendants while
rejecting a savepoint whose enclosing group was exited. The existing
lifetime-bound `ExecutionTransaction` remains useful inside recursive submodes
but does not replace this aggregate outer savepoint.

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
optimizations and never suspend `ExecutionRun`.

The `ResourceSite` recorded for diagnostics and failure injection is one of
`Expansion`, `MainControl`, `ParagraphFinish`, `LineBuild`, `PageBuild`,
`Shipout`, `FontLoad`, `ExternalImageParse`, or `EndFinalization`. Site does not
change request identity or atomicity:

| Site                                  | Contract on `Need`                                                                                                                                                                                                                                                                    |
| ------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| expansion                             | Roll back token acquisition, macro/scanner frames, input cursors, resolution index, paragraph reads, recoverable diagnostics, and all expansion fuel state except cumulative run fuel.                                                                                                |
| main-control dispatch                 | Roll back the delivered token, assignment/group mutations, mode changes, virtual effects, and candidate statistics; replay starts before expansion of that token.                                                                                                                     |
| paragraph finishing and line building | Roll back the entire paragraph-ending dispatch, paragraph memo validation/recording, line nodes, contribution/page-list changes, and any output it triggered. Pure line breaking itself must not call a host; resource-dependent font/shaping inputs are resolved before entering it. |
| page building                         | Roll back contribution consumption, page totals, insertion splitting, fire-up state, and candidate output work. Pure page-cost calculation has no resolver.                                                                                                                           |
| shipout                               | Roll back box removal, deferred-write expansion, stream state, image/font selection, detached effects, artifact bytes, prepared DVI plans, and page-node release. `ShipoutComplete` is staged only after the outermost shipout commits.                                               |
| font loading                          | Roll back request-index advancement and every partially scanned `\font` assignment. Parsing supplied bytes occurs during resource registration; selection is recorded in `World` only on replay.                                                                                      |
| external image parsing                | Registration validates and pins the external object first. A need during `\pdfximage` or shipout rolls back the whole containing step; no partial PDF object, dimension, or image ledger entry survives.                                                                              |
| end-of-job finalization               | Roll back final paragraph/page cleanup, output-routine work, shipouts, final summaries, and artifact-plan assembly. Replay re-enters `FinishEnd` or `Finalize`, never an internal output frame.                                                                                       |

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
   plan, checkpoint, paragraph memo observation, or read-recorder callback from
   a rolled-back candidate is visible.
4. Previously committed executor steps remain private but intact across a
   later suspension. Rejecting the whole compile discards the enclosing VFS
   build/revision transaction; accepting it publishes all committed steps at
   once.
5. A resource response never mutates TeX semantic state directly. It extends
   the immutable provisioner generation; the replayed TeX operation observes
   and records the selection at its original point in execution order.
6. A named checkpoint is captured only at the existing `JobStart`, eligible
   `OuterParagraphEnd`, and eligible outermost `ShipoutComplete` schedule. Step
   savepoints are private, unhashed, unretained after commit, and never offered
   for incremental restart.
7. Resource suspension cannot bypass a TeX group, shipout transaction, output
   routine, paragraph history barrier, or hard limit.
8. Successful stepwise output, statistics, checkpoint schedule, effects,
   artifacts, and generated files are byte-for-byte and order-for-order equal
   to the one-shot adapter with the same resources.

## Counters, fuel, cancellation, and hard limits

Counters fall into two classes.

Rollback-coupled counters include `ExecutionStats`, input source/replay and
condition allocators, `Universe` allocation and PDF cursors, effect/artifact
positions, paragraph recording ordinals, prepared-page occurrence counts,
checkpoint occurrence state, and the expansion `resolution_index`. Restoring
the savepoint restores these exactly. The replayed lookup therefore receives
the same resolution index; wraparound becomes a typed hard-limit failure
rather than the current wrapping behavior.

Monotonic counters are `advance_calls`, `suspension_serial`, response-progress
generation, failure-injection sequence, and `cumulative_fuel`. They are outside
the savepoint, never decrease on retry, and are telemetry or abuse-control
state rather than TeX semantics. Request identity never depends solely on one
of these counters.

Fuel is charged before each expansion loop action, delivered-token dispatch,
text-span token, memo validation unit, builder unit, shipped node/event, and
finalization unit. Work performed by a candidate that later rolls back remains
charged. This prevents a document from resetting its budget by causing
resource misses. The existing per-expansion fuel scope remains a local
recursive-expansion limit; the owned run adds a checked cumulative `u64` hard
limit for the logical candidate revision. Crossing either hard limit detaches
a typed error, rolls back the current step, and terminally fails that candidate.
It is not `AwaitingResources` and cannot be retried by increasing a limit.

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
session layer, cancelling a pending editor revision drops its `ExecutionRun`
and private VFS/revision transaction while preserving the last accepted
revision and immutable resource bindings.

Node, input-depth, recursion, output, generated-file, resource, and decoded
font/image limits remain hard terminal errors. A limit reached during a step
uses the same rollback protocol. Candidate bytes are counted before allocation
or publication; cumulative fuel is the sole limit intentionally not refunded
by rollback.

## Native, WASM, and build composition

Both hosts drive the same Rust `ExecutionRun` and typed result values. A native
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

## Migration sequence

The first migration step is implemented by the resolver-facing
`ResourceLookup<T>` contract shared by `tex-expand` and `tex-exec`. Resolver
calls now distinguish `Available`, authoritative `Unavailable`, and
`NeedResource(ResourceNeed)` outcomes, with malformed or host failures left in
the error channel. The current one-shot adapter still assembles public request
batches from its resolver-side request records and retries the whole candidate;
later steps replace only that outer retry policy, not the typed engine control
path.

1. Add typed `ResourceLookup`/suspension propagation to input, font, and image
   resolvers without changing one-shot behavior. Remove missing-resource
   detection through diagnostic strings.
2. Split `ExecutionContext` into owned `ExecutionState` and borrowed
   `ExecutionServices`; make the job name, expansion internals, resolution
   index, recoverable diagnostics, and paragraph memo state movable across
   calls.
3. Add the opaque, infallibly restorable `InputStackSnapshot` and an aggregate
   `StepSavepoint` over input, `Universe`, modes, execution state, stats, and
   private output/checkpoint staging.
4. Introduce `ExecutionRun`, `ExecutionStep`, and `ExecutionStepResult`; port
   `JobStart` and one-token/fixed-span `MainControl`, then make existing `run*`
   methods loop over it.
5. Make paragraph finish, page drain, shipout, and end/finalization return typed
   suspension unchanged through every recursive layer. Stage checkpoint and
   recorder delivery until step commit.
6. **Implemented.** Move prepared DVI/artifact suffix assembly into `Finalize`, add cumulative
   fuel and cancellation polls, and enforce checked counter exhaustion.
7. **Implemented.** `tex-incr` and `VirtualCompileSession` retain an
   `ExecutionRun` across resource batches instead of rerunning a cold or
   pending revision. The enclosing candidate revision, speculative engine
   roots, and private VFS build generation remain owned by the candidate.
8. Route native CLI, direct WASM, worker, and authored JavaScript loops through
   the same session results; remove the full-attempt resource-retry path after
   parity and failure-injection gates pass.

Steps 1--6 belong primarily to `tex-expand`, `tex-lex`, `tex-state`, and
`tex-exec`; step 7 composes `tex-incr`, `umber-vfs`, and `umber`; step 8 removes
adapter-specific retry assumptions. Each migration step must leave the
one-shot adapter passing before the next begins.

## Focused tests and failure injection

Unit tests inject `Need`, cancellation, and hard failure at deterministic
`(ExecutionStep, ResourceSite, operation_ordinal)` points. Every injection
compares a before/after aggregate projection containing input summary and
transient replay, mode summary, `Universe` state hash, group depth, page state,
effect/artifact positions, stats, expansion index, paragraph recorder, pending
fire-up, prepared pages, and staged generated-output prefix.

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
  writes, generated file writes, an artifact, a prepared DVI page, a paragraph
  memo hit, and cold paragraph dependency recording have each been produced;
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
