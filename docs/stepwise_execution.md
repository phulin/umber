# Stepwise TeX execution and resource replay

Status: approved `umber2-du4r` architecture. The host replay and ordinary
parser implementation are coordinated with the resource/readiness and parser
work on Beads issue `umber2-du4r`; implementation status belongs there.

The resource-boundary contract is [Resource-miss checkpoint replay](checkpoint_resource_replay.md).
This document describes the engine/session lifecycle around that contract. It
supersedes the older stepwise design that retained a typed scanner,
expansion, caller, or `OperationFrame` continuation and resumed only the
rolled-back operation.

## Boundary and ownership

`MainControl` runs ordinary synchronous parser calls until a semantic boundary,
terminal result, or resource need. Raw and expanded token delivery, macro
expansion, numeric and expression scanning, output routines, and alignment
work use local Rust calls plus the language-semantic input, macro, group,
conditional, alignment, and bounded expression stacks. Those stacks model
TeX; they are not parked host continuations.

When a lookup cannot continue, the entire active call tree unwinds. The engine
returns a cold owned `ResourceNeed` with a complete semantic request, intent,
site, and any detached diagnostic source location. It does not return a
borrowed cursor, caller edge, scanner frame, expansion frame, output frame,
future, resolver, or executor-local resume token. A low-level `step` function
must not promise to resume an unwound call on the same executor.

The host session owns the retry. It selects a `ResourceReplayAnchor`, restores
the full aggregate checkpoint and matching host transaction prefixes, installs
the admitted outcome through the ordinary VFS/World path, and starts ordinary
main control again. This is one host replay protocol for native and WASM; the
hosts differ only in acquisition and transport.

The concrete engine seam discards the active direct `MainControl` operation
when a resource need escapes and returns only a detached, owned `ResourceNeed`
and any detached diagnostic provenance. The retained candidate restores its
full command, mode, execution, statistics, checkpoint, page, effect, artifact,
and lifecycle roots before entering `AwaitingResources`; the next drive creates
a fresh control operation. No completed operand, scanner cursor, expansion
frame, caller edge, or prepared-operation continuation crosses the host
boundary, and a stale direct drive is rejected rather than resumed.

The expanded path remains one fast iterative kernel. The canonical numeric and
expression scanners preserve signs, internal values, parentheses, and exact
terminators. Ordinary macro expansion remains iterative. Resource handling
must not add a dispatch/state-machine transition per token or a second parser.

## Session lifecycle

```text
Created
  | ordinary synchronous run
  v
Running ------------------------------> Complete
  | resource need                         ^
  v                                       |
AwaitingResources -- provision/replay ---+
  | cancellation, failure, or no progress
  v
Failed / Cancelled
```

`AwaitingResources` is a host state, not an engine continuation. The session
retains the request/cache/outcome ledger, cumulative work, and at most one
current-candidate replay anchor. It does not retain an active scanner or
caller stack. An empty or partial host response may proceed only when normal
request-progress rules permit it; repeating an already bound request is a
typed no-progress failure.

One-shot adapters may drive the same session loop to completion. Persistent
editor and project sessions retain the accepted revision and private current
candidate, but a resource retry restores from the selected full checkpoint.
They do not claim that a resource response resumes the exact Rust call that
asked for it.

The public API names used by an implementation are deliberately not fixed by
this document. In particular, do not add a same-executor `CanonicalStepRunner`
or typed scanner-resume API merely because an older version of this document
described one. Any existing compatibility adapter must translate to the host
replay protocol and remain outside parser state.

## Checkpoint selection

The retry anchor is the newest valid _full_ checkpoint under existing
eligibility:

- an eligible outer-paragraph checkpoint in the main file, subject to its
  existing source revision and edit-position checks; or
- the initial format/`JobStart` fallback, represented by the immutable
  `FrozenJobStartAnchor` when no paragraph checkpoint is available.

There are no per-command checkpoints, no new eligibility rule, and no capture
of active scanner, output, alignment, or partial-call state. The checkpoint
schedule and editor edit-restart boundary do not change for resource replay.

The current candidate owns at most one `ResourceReplayAnchor`. The anchor
contains an existing full `EngineCheckpoint` and cheap host marks for the
generated-file, output/effect, and diagnostic transaction prefixes that belong
to the same source revision and output state. It reuses aggregate
checkpoint/fork/restore ownership and keeps only the prior/current generation
lineages. It never copies whole engine state, keeps a third history lineage,
or allocates a checkpoint per request.

On a miss, the host:

1. unwinds and discards the failed direct `MainControl` operation, detaching
   the owned need and any source origin that must survive the attempt;
2. restores input, macro/group, conditional/alignment, mode, world, page, PDF,
   dependency, provenance, and source roots from the full checkpoint;
3. rewinds output/effect and diagnostic ledgers to the matching host prefixes;
4. drops post-anchor input, observer, output, generated-file, and other effect
   suffixes; and
5. runs a fresh ordinary main-control operation with the admitted outcome
   visible in VFS.

The request may have been discovered inside expansion, a scalar or structured
scanner, alignment, page building, an output routine, shipout, or finalization.
The containing call tree is still discarded; the anchor is the same. No
resource site gets a private continuation representation.

Candidate rejection or cancellation drops the anchor and all current-generation
rows together. An accepted ancestor remains available only if it satisfies the
existing edit-position rule. A diagnostic origin that must outlive the
candidate is detached to an owned source location before candidate drop.
External writes and publication occur once, after acceptance.

## Resource protocol

The engine's typed request protocol distinguishes required inputs, blocking
existence probes, and optional prefetch hints. Required and probe responses
may be positive or authoritative absence; hints are suggestions and never
create negative bindings or retry progress. Access, transport, validation, and
cache failures are actionable errors, not absence.

The host keeps outcome and readiness state outside rollback:

```text
Fulfilled            verified bytes/metadata admitted to engine-readable VFS
AuthoritativeAbsent   canonical scoped negative answer
Pending              acquisition or engine admission is incomplete
FetchError           access, transport, validation, or provider failure

Ready                record and required payload/metadata admitted
ExistsNotReady       canonical record exists but admission is pending
Absent               canonical scoped negative
```

Catalog lookup, `FileKind`, extension defaults, provider precedence, and
project/generated/distribution layering remain the existing canonical
resolver. A complete pinned immutable catalog is available before execution.
Native disk-cache and WASM IndexedDB hits still need host admission into the
engine-readable store. Metadata probes do not force payload downloads.
Distribution negatives are scoped to the pinned root, project negatives to
the frozen source revision, and generated negatives to the current generated
transaction; rollback or writes invalidate the last kind. Arrival of a
speculative payload cannot change search precedence.

## Startup preflight

A shared one-shot host preflight may emit hints before the engine starts. It is
not a checkpoint restart; an empty speculative response proceeds normally. Its
bounded baseline is:

1. the last accepted run's attempted-and-successful lookup manifest, keyed by
   engine/profile, format, options, authenticated distribution, and
   provider/search policy while allowing document-text edits;
2. literal-only `documentclass`, `usepackage`, `RequirePackage`, `input`, and
   `includegraphics` hints through the canonical resolver;
3. small package runtime/dependency groups from existing packed metadata; and
4. bounded escalation of demanded resources and relevant small groups after
   repeated expensive replay in one region.

Hints do not expand macros or interpret TeX. Recursive inspection of already
fetched small runtime text is bounded and deduplicated, with separate budgets
for document, font, and image data. Required requests always win. The startup
set is made ready before a run and the demanded set before a retry. A
successful manifest is published only after accepted output/generated state;
failed-attempt discoveries can schedule work but cannot replace it.

## Accounting and atomicity

Fuel, execution work, response progress, and session counters are monotonic.
Every actual resource restart, including one caused by external provisioning,
is counted. Rewinding semantic state never refunds discarded work, and a
fully admitted run is not required to have the same fuel as a replayed run.

The following are semantic invariants:

1. A resource miss leaves no scanner, expansion, caller, output, or host
   capability continuation in the engine.
2. Full checkpoint restoration plus the matching host prefixes removes every
   post-anchor input, world, page, PDF, diagnostic, observer, effect, output,
   and generated-file suffix.
3. Replaying with the same admitted resources is independent of host response
   order and batch partitioning and is equivalent to a fully preloaded run.
4. External writes, diagnostics detached for the host, artifacts, and generated
   files are published at most once after safe acceptance.
5. A response cannot mutate semantic state directly; replay observes it at the
   original lookup through ordinary VFS/World registration.
6. Candidate discard drops its anchor and current lineage; no continuation
   crosses into the accepted prior lineage.
7. Fuel and discarded-work metrics remain monotonic across repeated misses.

Telemetry reports actual resource restarts, cold versus repeated rounds,
replayed/discarded CPU or fuel, demand bytes, prefetched bytes, unused
prefetch bytes, wait time, and peak memory. A file-hit rate alone is not
sufficient evidence.

## Required review cases

The combined implementation review compares fully admitted and injected-miss
runs for diagnostics, generated files, effects, artifacts, and output. It
covers misses in nested scanners, output and alignment work, true absence,
provider/access failure, changed project negatives, precedence, current
candidate anchor retention, and candidate discard. It also proves that
repeated identical runs improve readiness without changing semantics. Reduced
bootstrap measurements use the same optimized build and active progress
counters; no full format regeneration is needed for this proof.
