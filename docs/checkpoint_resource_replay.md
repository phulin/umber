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

On replay the host:

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
   VFS/World path.

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

`FetchError` is actionable failure, never `AuthoritativeAbsent`. A fulfilled
or authoritative-absent response is staged once outside rollback, the candidate
is rewound, and replay observes that answer at the original lookup. A pending
request returns to the host. A response for a cancelled or discarded candidate
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

## Startup preflight

One shared host preflight may emit startup hints before the engine begins. It
is not a checkpoint restart and an empty speculative reply must proceed to the
engine. The baseline sources, in order, are:

1. the last accepted run's attempted-and-successful lookup manifest, keyed by
   engine/profile, format identity, options, authenticated distribution, and
   provider/search policy while allowing document-text edits;
2. literal-only `documentclass`, `usepackage`, `RequirePackage`, `input`, and
   `includegraphics` hints resolved through the canonical resolver;
3. bounded small package runtime/dependency groups from the existing packed
   metadata; and
4. a bounded escalation of the demanded resource and relevant small group
   after repeated expensive replay in the same region.

Literal hints never interpret macros or execute TeX. Recursive hint discovery
may inspect already-fetched small runtime text only with bounded deduplication
and separate font/image/document budgets. Hints are suggestions, not semantic
input. Required lookups always win, and absence of a speculative hint never
creates a negative binding.

`umber-distribution::PrefetchPolicy` is the one policy owner for both native
and WASM. It owns literal extraction, canonical `FileRequestKey` queue
deduplication, selection budgets, bounded runtime closure, and replay-region
escalation. `PrefetchPlanner` and the WASM DTO binding are adapters; native
resolution and browser JavaScript retain only transport, provider ordering,
and the original spelling/search context needed to issue a request. The shared
defaults are 64 files/16 MiB total, 8 MiB small-runtime, 2 MiB font and image,
512 KiB document, 256 KiB scanned runtime text, 32 follow-up hints, and one
follow-up tier.

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

The host confirms the startup set's readiness before the run and the demanded
set's readiness before retry. A successful lookup manifest is published only
after accepted output and generated state commit. Failed-attempt discoveries
may schedule work but cannot overwrite that manifest. Native and WASM share
the semantic manifest and readiness policy; acquisition and cache transport
remain platform-specific.

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
