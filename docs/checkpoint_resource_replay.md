# Resource-miss checkpoint replay

Status: approved architecture for `umber2-du4r`. The implementation wave is
still subject to the combined host, resource, and parser review recorded on
Beads issue `umber2-du4r`; this document is the contract, not a validation
receipt.

This document supersedes every older recommendation to park a scanner,
expansion, caller, output-routine, or operation continuation at a resource
boundary. In particular, a low-level stepping API that resumes an unwound
Rust executor is not a safe same-executor resource-retry interface. The host
session owns replay.

## Ordinary engine boundary

Raw and expanded scanning use ordinary synchronous Rust calls and local
variables. The language-semantic stacks remain real engine state: input and
macro input, groups, conditionals, alignments, and bounded expression/scanner
state are still required by TeX. They are not resource continuations.

An unavailable resource unwinds the complete current call tree. The engine
returns a cold, owned `ResourceNeed` containing the semantic request, request
intent/site, and any diagnostic origin that must survive the unwind. No parked
scanner, expansion frame, caller return edge, output frame, or Rust reference
is retained. A resource response is never applied by a half-unwound parser.

The expanded path remains one fast iterative kernel. Numeric and expression
scanners remain the canonical scanners, including their sign, internal-value,
parenthesis, and exact token-termination behavior. Ordinary macro expansion
stays iterative. Resource handling must not introduce a per-token dispatcher,
state-machine transition, or alternate scanner implementation.

## Full replay anchor

The retry point is the latest valid _full_ `EngineCheckpoint` permitted by the
existing checkpoint schedule:

- an eligible outer-paragraph checkpoint in the main file, with its existing
  edit-position and source-revision rules; or
- the initial format/`JobStart` fallback, represented by the immutable
  `FrozenJobStartAnchor` when no paragraph checkpoint is eligible.

There are no per-command checkpoints, no new eligibility rule, and no capture
of active scanner, output, alignment, or partial-call state. A resource miss
does not change the editor edit-restart boundary.

The host retains at most one current-candidate `ResourceReplayAnchor`. It is a
cheap owner of the full checkpoint plus the host-side generated-file,
output/effect, and diagnostic transaction-prefix marks needed to restore the
same externally visible prefix. It reuses the existing aggregate
checkpoint/fork/restore machinery; it does not copy the whole engine, create a
third lineage, or retain a checkpoint per request.

When the legacy outer host path receives a resource response, the host:

1. unwinds and discards the failed direct `MainControl` operation, detaching
   the owned need and any source origin that must survive the attempt;
2. restores input, macro/group state, condition and alignment state, mode,
   world, page, PDF, dependency, provenance, and source roots from the full
   checkpoint;
3. rewinds the current output/effect ledger to the anchor's sealed prefix;
4. drops input, observer, diagnostic, output, generated-file, and other
   post-anchor suffixes; and
5. stages the admitted outcome after the full restore, then starts a fresh
   ordinary main-control operation with it visible through the normal
   VFS/World path. Provider-capable command execution has a separate
   synchronous path described below: a ready or unavailable answer is
   installed during the current cold call, while only a declined answer uses
   this full-replay sequence.

The checkpoint and every retained host prefix must describe the same source
revision, generated-output transaction, and output state. An accepted ancestor
checkpoint remains subject to the existing edit-position rule. Replaying in
place preserves only the prior accepted and current candidate lineages. If the
candidate is rejected or cancelled, its anchor and current-generation rows are
dropped together; the accepted history is the only fallback outside it.

External writes, terminal effects, generated files, artifacts, and observer
publication occur once, after safe acceptance. A diagnostic `OriginId` that
must outlive a dropped candidate is detached to an owned source location
before the candidate generation is released; a runtime origin handle never
crosses that boundary.

## Resource outcomes and readiness

Requests, verified cache objects, readiness state, and outcomes live outside
engine rollback. The host distinguishes at least these outcomes:

```text
Fulfilled            verified bytes/metadata admitted to engine-readable VFS
AuthoritativeAbsent   canonical, scoped negative answer
Pending              request is still being acquired or admitted
FetchError           access, transport, validation, or provider failure
```

`FetchError` is actionable failure, never `AuthoritativeAbsent`. On the legacy
outer fallback path, a fulfilled or authoritative-absent response is staged
once outside rollback, the candidate is rewound, and replay observes that
answer at the original lookup. On the provider path, those same outcomes
continue the current operation after capability installation. A pending request
returns to the host. A response for a cancelled or discarded candidate
may remain in a shared verified cache, but it is not installed into that
dropped execution. A direct operation never resumes after its need escapes.

The public host protocol represents `FetchError` as
`ResourceOutcome::Failed(ResourceFailure)`. `ResourceOutcome::Declined` is
reserved for a genuinely pending request; a failure is returned through the
session/incremental error boundary immediately and is never retried internally,
converted into `NoProgress`, or recorded as a negative binding.

Canonical catalog lookup, `FileKind` identity, default extensions, provider
precedence, and project/generated/distribution layering remain the existing
resolver's authority. Do not add a second resolver or JSON catalog. A complete
pinned immutable catalog is available before execution; existence and payload
readiness are separate:

```text
Ready            canonical record and required bytes/metadata admitted to VFS
ExistsNotReady   canonical record exists, admission or payload is pending
Absent           canonical scoped negative
```

Native disk-cache and WASM IndexedDB hits are not engine-ready merely because
the host cache can read them. Required payload or metadata must cross the host
admission boundary into the engine-readable store before a run or replay.
Existence/metadata probes do not force payload downloads. Distribution
negatives are scoped to the pinned distribution root; project negatives to the
frozen project source revision; generated negatives to the current generated
transaction and are invalidated on rollback or write. Payload arrival never
changes winner or search precedence.

### Synchronous resolution of resident resources

An already host-resolvable resource is settled at the cold command site in
the same ordinary Rust call. `CommandHostContext` carries an optional,
borrow-scoped `ResourceProvider`; its `resolve` method receives the live
`CommandContext` and an owned-neutral `ResourceNeed`, and returns an owned
fulfilled or unavailable answer with the dependency reads that produced it.
The provider borrow ends before the command resumes, so no provider, world,
scanner, or caller borrow can enter a checkpoint or parser state.

The command crate owns the canonical request, fulfillment, outcome, failure,
and dependency-effect vocabulary. A single capability installer validates the
typed request/answer pair, installs the binding or scoped negative, and
records the dependency reads. The executor's fallback ledger retains its
existing acknowledgement at the same call site; replay and startup fallback
call the installer as well and do not maintain a second answer protocol.
When a tex-exec adapter invokes the legacy `ResourceHost`, it exposes only a
closure-scoped `InputReadState` view of the live World. World effects recorded
by that call are already present in the speculative candidate and are not
applied a second time when the capability is installed.

Installed input and font capabilities retain immutable selected bytes and
their semantic metadata after the active World input record is stripped.
They may share a non-authoritative `InputRecordId` cache hint so a repeated
use can reuse a live record without another backing read. World validates that
hint against the current timeline and exact selected bytes, refreshes it when
the record is replaced, and registers a new record when it is stale. The hint
is metadata only and cannot retain World state across a checkpoint or rollback.

An unavailable answer continues the existing TeX diagnostic or optional-file
path immediately. A failed answer propagates its original failure. A declined
answer carries one call-scoped already-attempted marker to the outer driver;
that marker suppresses exactly one duplicate host call while the operation
unwinds, then is consumed. It is neither part of request equality nor a
persistent cache or generation identity. A subsequent drive may ask the host
again.

Resource producers construct their complete typed request before capability
lookup. Input size/date/MD5/dump/openin probes retain `AuthoritativeProbe`
dependencies, while a later required input read records `RequiredRead`; a
probe binding never promotes itself to input backing. Font requests retain
their full target, name, size, recovery, and diagnostic context. Image
requests retain the live pdfoutput/pagebox/resolution/dimensions/colorspace
identity, with attempt-local attributes excluded as required by
`same_resource_as`.

## Startup preflight

One shared host preflight may emit startup hints before the engine begins. It
is not a checkpoint restart and an empty speculative reply must proceed to the
engine. The baseline sources, in order, are:

1. the last accepted run's attempted-and-successful lookup manifest, keyed by
   engine/profile, format identity, options, authenticated distribution, and
   provider/search policy while allowing document-text edits;
2. literal-only `documentclass`, `usepackage`, `RequirePackage`, `input`, and
   `includegraphics` hints resolved through the canonical resolver, plus
   bounded literal `DeclareFontShape` TFM hints for direct or statically scaled
   font names;
3. authenticated dependency metadata attached to an admitted seed, which the
   planner may enqueue only after that seed is engine-readable; and
4. a bounded escalation of the demanded resource and relevant small group
   after repeated expensive replay in the same region.

Literal hints never interpret macros or execute TeX. `DeclareFontShape` TFM
extraction accepts only the direct or literal numeric-scale forms of its
balanced declaration; aliases, dynamic names/scales, size selectors, trailing
expressions, and malformed or nested controls are skipped. Recursive hint
discovery may inspect already-fetched small runtime text only with bounded
deduplication and separate font/image/document budgets. Hints are suggestions,
not semantic input. Required lookups always win, and absence of a speculative
hint never creates a negative binding.

`umber-distribution::PrefetchPolicy` is the one policy owner for both native
and WASM. It owns literal extraction, complete semantic file-key
deduplication, selection budgets, bounded runtime closure, and replay-region
escalation. A policy request carries `domain`, `kind`, normalized `name`,
original spelling, and search context separately from its catalogue transport
key; a budget class is only an accounting class and never reconstructs a file
kind. The WASM DTO serializes those semantic fields explicitly, so a shared
`tex:<name>` payload can satisfy distinct VF, PDF, image, or asset admissions
without aliasing their request identities. `PrefetchPlanner` and the WASM DTO
binding are adapters; native resolution and browser JavaScript retain provider
ordering and issue the original typed request. The native planner drains
high-confidence startup and literal closure waves before broad metadata peers,
while one phase reservation spans every such wave. The shared defaults are 64
files/16 MiB total, 8 MiB small-runtime, 2 MiB font and image, 512 KiB
document, 256 KiB scanned runtime text, 32 follow-up hints, and one
follow-up tier.

The planner retains a typed discovery context `(origin, depth)` for each
selected semantic `PrefetchFileKey` while it crosses the request, resolver,
VFS-admission, and policy boundaries. Source, explicit, prior-observed, literal,
and actual-demand requests are roots; literal children retain their scanner
depth, and a child increments that depth exactly once. An authenticated metadata
dependency is a leaf for further metadata-peer traversal. If its bytes are
lexically scanned, the inherited depth and existing follow-up limit still
apply, but metadata admission cannot re-root its catalogue peers. A later
actual demand promotes the same key to a root. The context is merged by the
strongest origin, carried through selection and phase deferral, and retired on
admission or failed/absent speculative resolution; it is not a request payload,
cache entry, or unbounded history.

The byte, file, and class limits are one reservation owned by the current
automatic prefetch phase. A phase begins for startup preflight and for each
new actual engine `NeedResources` suspension. It remains live while its
required response is resolved, admitted, and its post-admission closure waves
are drained; a `provide_resources` call or another closure wave does not reset
it. Every selected semantic key is reserved once, while a payload identified
by its object, digest, and declared length is reserved once even when it
serves multiple semantic admissions. Required demand is tracked
independently and never consumes speculative ceilings. Queue insertion is not
a second file reservation: an unselected optional request remains pending or
deferred until a later phase, and an optional failure is never represented as
engine readiness. Only a genuinely new demanded resource starts another
phase; direct required demand may still request a previously declined key.

The admission callback runs only after a verified response has successfully
crossed the engine VFS transaction. It receives the canonical key, retained
path, admitted bytes, and packed dependency hints; only then may the policy
scan small runtime text and enqueue its next bounded batch. Catalog-positive
metadata is reported separately as `ExistsNotReady`; a cache hit or catalog
record never becomes `Ready` without engine admission. Empty or declined
speculative batches are acknowledged once and terminate, while required
responses and their original failures remain authoritative.

For repeated misses, the host supplies the opaque key of the actual retained
checkpoint region together with monotonic discarded-work telemetry. The policy
keeps per-run region and request diagnostics, and escalates known small
package/dependency companions by bounded tiers after additional discarded
work. It resets those counters for a new run/context and never substitutes a
suspension serial, creates a checkpoint, or fetches a whole distribution.

The host confirms each selected set's readiness before spending the next
closure wave or retrying the engine. A successful lookup manifest is published
only after accepted output and generated state commit. Failed-attempt
discoveries may schedule work but cannot overwrite that manifest. Native and
WASM share the semantic manifest and readiness policy; acquisition and cache
transport remain platform-specific. Unselected optional requests stay
pending/deferred and are never reported `Ready` or `Unavailable` merely
because the phase cap was reached.

## Accounting and ownership

Session fuel and work are monotonic. Rewinding semantic state never refunds
work, and the host counts every actual restart, including a restart whose
resource was supplied by external preflight/provisioning. There is no fuel
parity requirement between a fully admitted run and a run that discarded work.

Telemetry must expose actual resource restarts, replayed/discarded CPU or
fuel, cold versus repeated rounds, demand bytes, prefetched bytes, unused
prefetch bytes, wait time, and peak memory. A file-hit rate alone is not a
resource-replay metric.

The request/cache/outcome owner may outlive the current candidate. The engine
owns only admitted semantic bytes and rollback-safe dependency observations;
the host owns transport, cache, readiness, scheduling, and replay-anchor
selection. This keeps external effects one-shot while retaining at most the
prior accepted and current candidate engine lineages.

The standalone `umber::EngineSession` exposes the same ownership through its
public wait/fulfill protocol. A `NeedResource` result leaves the engine
operation unwound and records only the cold request. `fulfill` validates that
request and retains the typed answer and resolver effects; it does not mutate
the invalidated command machine. The next `advance_until_waiting` borrows the
latest valid full checkpoint from the caller's checkpoint sink, restores the
aggregate roots and matching output prefix, installs the answer, and starts a
fresh ordinary operation. A sink that does not retain a valid full checkpoint
cannot drive a resource retry. When the synchronous provider has already
declined a request, the session performs that same restore without calling the
host again; the next fresh operation is the one that may ask the host again.
No resource response, observer record, terminal effect, or generated output
is published before the restored retry commits.

## Required comparison

For the same admitted resource snapshot, fully preloaded and injected-miss
runs must produce identical diagnostics, generated files, effects, artifacts,
and output after replay. Focused misses must cover nested scanners, output and
alignment work, true absence, transient/provider failure, changed project
negatives, and precedence. The replay path must demonstrate that no scanner or
caller continuation survives candidate discard and that repeated identical
runs improve readiness without changing semantics. Reduced bootstrap
measurements use the same optimized build and active progress counters; they do
not require regenerating the full format or silently moving the boundary.
