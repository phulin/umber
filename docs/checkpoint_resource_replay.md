# Resource-miss checkpoint replay

This document is the ownership contract for host resource misses in retained
execution. It applies to native sessions and to the WASM session wrappers,
which share the incremental session runner.

## Boundary

The canonical engine executes ordinary synchronous scanners. A
`ResourceNeed` is an external transaction boundary: the current command
attempt is cancelled, the request and its detached source origin leave the
candidate, and no scanner, expansion, caller, or output-routine continuation
is parked for the host.

The retry anchor is the newest valid full `EngineCheckpoint` owned by the
current candidate. It is either an eligible outer-paragraph checkpoint or the
candidate's initial JobStart checkpoint. The immutable
`FrozenJobStartAnchor` remains the fallback for a cold candidate. Checkpoints
are never captured per command and eligibility is unchanged.

## Ownership and rollback

An anchor is a move-only key into the candidate's existing retained generation.
The candidate owns at most one current replay anchor in addition to its normal
history rows. Replaying rewinds the current generation in place:

1. discard the active direct `MainControl` operation and detach the owned need;
2. restore command, mode, world, page, PDF, dependency, and source roots from
   the aggregate checkpoint;
3. rewind the current output-ledger fork to the checkpoint's sealed mark; and
4. discard generated, diagnostic, observer, and other effect suffixes after
   the checkpoint prefixes.

The operation uses the existing prior/current physical generations. It never
creates a third lineage or copies the whole engine. If a candidate is dropped,
its anchor and current-generation rows are dropped with it; the accepted
history remains the only fallback outside the candidate.

## Host outcomes

The request, cache state, outcome, and detached origin are owned outside the
rollback. A fulfilled or authoritative-unavailable response is staged exactly
once, the candidate is rewound, and the staged outcome is applied after the
full restore, before a fresh synchronous scanner call encounters the same
need. A pending request yields to the host and is retried only through that
checkpoint replay. The host-world adapter records only semantic dependency
observations as replay effects, so those observations are restored alongside
the answer without repeating the host fetch. Fetch errors are reported as
errors rather than being turned into authoritative absence. Resource bytes and
external publications are therefore never duplicated by a replay.

Fuel, execution budget use, discarded work, and replay counters are monotonic;
rewinding semantic state does not refund session work. Native CLI,
`CompilerSession`, `EditorSession`, and `LatexProjectSession` all observe the
same replay boundary through `VirtualCompileSession`. The VFS generated-file
transaction is discarded at each host boundary and reopened from the retained
candidate workspace, so generated writes and negative bindings are never
published by a failed attempt.

## Metrics

Execution telemetry reports resource restarts, replayed dispatches, replayed
delivered commands, and discarded fuel separately from ordinary host
suspensions. Retention metrics continue to account for accepted checkpoint
history. A live candidate keeps its JobStart replay root protected; terminal
completion releases that runtime root before publication while retaining
detached boundary evidence, and rejection drops the candidate root with the
current generation.
