# Private revision allocation domains

Status: ownership and lifecycle foundation for Beads issue `umber2-3v8z.2`.

This document defines the allocation boundary used by private incremental
revisions. It complements the semantic identity contract in
[Core Engine State](core_state.md), the retry boundary in
[Stepwise Execution](stepwise_execution.md), and the acceptance boundary in
[Incremental engine v1](incremental_v1.md).

## Invariant

Persistent memory is reachable semantic state, live rollback authority, or
detached published output. A private revision owns every allocation made while
executing that revision. An executor operation owns only a suffix of the
revision domain:

```text
accepted roots
  -> private revision domain
       -> operation mark
            success: keep the suffix in the private revision
            failure or NeedResource: truncate the suffix exactly
       -> rejection: drop the complete private domain
       -> acceptance: transfer explicit rooted objects, then drop the domain
```

Allocation coordinates, domain identity, operation serials, slot-vector
capacity, and transfer bookkeeping are operational authority. They do not
participate in semantic hashes, formats, checkpoints, effects, artifacts, or
output identity.

## Owners

`tex-state::Universe` owns the one domain attached to a private revision. The
domain is absent from templates and accepted generations at rest. Beginning a
candidate installs a fresh domain after a cold clone or validated checkpoint
fork. A domain never crosses into another candidate and no process- or
session-wide registry records old domains.

`Stores` and `World` remain reachable only through `Universe`. Their current
watermarks, persistent roots, effect positions, and journals stay the semantic
rollback substrate. Reachability-owned value stores allocate immutable
payloads in the revision domain. Node construction uses an operation-local
`NodeListBuilder`; freezing returns a structural `NodeListRef`, and commit
moves that reference into the mode, page, control, Env, checkpoint, or output
aggregate. Retry and rejection drop scratch references normally. The domain
does not traverse stores, reconstruct indexes, or compact historical
allocations.

`tex-command::CommandState` owns command cursors and typed resource
continuations, including the sole ordinary command-attempt coordinate; the
executor moves only its coordinate-free lifecycle edge. A genuine suspension
may retain the coordinate in its pending package for owner-exact admission. It
contains no allocation-domain control. `tex-exec::MainControl`
opens one fixed-size `Universe::DirectOperationMark` after preflight. Successful
operation commit closes the private suffix without releasing earlier work.
Ordinary failure and cancellation discard only unpublished operation
allocations; resource suspension retains the fully prepared continuation and
does not restore aggregate command, state, or mode roots.

`tex-incr::RevisionCandidate` owns the `Universe`, command state, speculative
checkpoints, and detached candidate output across resource suspensions. A
suspension retains that one candidate and its earlier committed private work;
it does not create a second allocation domain or retain the failed operation's
suffix. Dropping the candidate rejects the domain.

`RevisionTransaction` is still private revision ownership after execution has
completed and while detached output is validated. Dropping it rejects the
domain. `Session::accept_revision` is the only publication boundary. A
replacement candidate transfers explicit live roots into accepted ownership;
a converged candidate transfers only the new roots required by the accepted
detached prefix, then drops its scratch domain. Neither path retains the
domain merely because one allocation survives.

Node-list payloads need no allocation-domain transfer ledger. A private
candidate's mode, page, command, Env, PDF, checkpoint, and output staging
values own their `NodeListRef` fields directly. Success moves those references,
resource retry retains its typed continuation, rejection drops the candidate
closure, and acceptance moves the selected aggregate. Detached format, memo,
DVI, PDF, and HTML values contain no runtime
node coordinate that the domain could retain.

## Operation marks

One domain has at most one active aggregate operation mark. Main control owns
all nested scanner, alignment, paragraph, page, and shipout work beneath that
mark, so a second independent mark would split rollback authority. A mark is
owner-exact, single-use, and serial-checked. Foreign, stale, nested, or
out-of-order marks fail before mutation.

Rollback removes every domain slot allocated after the mark and restores the
exact logical byte charge. It also releases unused slot capacity above the
restored extent, so repeated large failed operations cannot make the domain's
retained allocation metadata follow retry count. Commit retains the suffix
once as part of the private revision. It does not copy the payload or mint a
new handle.

The existing provenance/source-map retry compatibility behavior is separate.
It may preserve verified retry identities in those legacy stores until the
provenance migration, but it grants no exception to patch-domain truncation.

## Root transfer

Domain payloads are individually reference-counted immutable objects. Runtime
handles are non-owning, owner-checked coordinates. Acceptance receives an
explicit deterministic root list from the aggregate owners, validates every
root before transfer, moves ownership only for the distinct listed payloads,
and consumes the domain. Unlisted payloads are released with the domain.

An accepted payload may structurally own other immutable payloads. That
ordinary reference ownership, established while building the value, is its
reachability closure. Acceptance does not walk an object graph, copy an arena,
trace handles after execution, or discover roots from hashes. A future store
migration must expose its complete typed live-root projection at the mutation
boundary where those roots are already known.

Named checkpoints remain explicit rollback roots. Detached effect and artifact
values remain independently owned output. When a migrated allocation is
reachable from either, its owner must be included in the typed transfer set or
already held structurally by that root. Cache membership and probabilistic
identity never confer ownership.

## Lifecycle controls

Focused controls must prove:

- operation rollback restores allocation count, logical bytes, and root
  liveness exactly;
- a retry retains successful earlier private allocations once while dropping
  every allocation from the blocked operation;
- rejecting a candidate or prepared transaction releases its complete domain;
- accepting one root releases unrelated allocations and keeps the rooted
  payload readable after the domain is gone;
- duplicate roots transfer one owner, foreign and stale handles fail closed,
  and no handle keeps a rejected domain alive;
- checkpoints and detached output preserve their explicit roots; and
- repeated accept, reject, and retry sequences plateau at the live root and
  operation high-water size, independent of patch count.

State, effect, artifact, command-summary, checkpoint schedule, and
allocation-independent exact identity must match clean execution. The controls
use exact logical byte charges and bounded allocator observations; they do not
weaken memory guards or use a corpus/profile run as a substitute.

## Deliberate exclusions

Output ledgers and caches remain outside this contract. Structural node
ownership adds no copying compactor, post-hoc live-state graph traversal,
historical-generation registry, cache budget, or document-specific
reclamation rule.
