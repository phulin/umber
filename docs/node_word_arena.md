# Structurally Owned Compact Node Lists

## Status

All production node-list lifetimes are represented by `NodeListRef`. Page,
mode, command, alignment, box, PDF, checkpoint, generation, retry, revision,
and shipout aggregates store `NodeListRef` directly, either as fields or inside
owned `Node` values. There is no epoch arena, survivor arena, promotion table,
pin journal, root slot, refcount ledger, or raw-coordinate lifetime API.

## Representation

A `NodeListRef` is a strong reference to one immutable `NodeListPayload` plus a
compact span inside that payload. The payload contains canonical node words,
typed sidecars, semantic spans, and every nested list reachable from the root.
Cloning the reference clones structural ownership; dropping the final clone
releases the whole payload.

`NodeListId` is not an owner. It is a private compact coordinate used while a
`NodeListRef` is borrowed, in packed node sidecars, and as a dense detached
encoding key. Resolving a child coordinate requires the enclosing
`NodeListRef`; a coordinate cannot upgrade or rediscover a dropped payload.

The canonical empty list is also a `NodeListRef`. Empty child coordinates
resolve without consulting global state.

## Construction

`NodeListBuilder` owns ordinary `Node` values and records their strong child
references. Freezing performs these steps atomically:

1. validate non-node handles and direct child ownership;
2. compute allocation-independent semantic identity;
3. encode the root and its child payloads into one immutable compact graph;
4. return one `NodeListRef` for the root span.

Validation failure publishes nothing. A weak, bounded candidate index may
reuse an exactly equal live payload, but weak entries neither retain payloads
nor recover dead ones.

## Aggregate transitions

Every transition follows ordinary structural ownership:

- success moves a reference into the destination aggregate;
- committed failure keeps references already present in the canonical partial
  state and drops failed scratch values;
- rollback replaces the live aggregate with its cloned snapshot, dropping the
  rejected references;
- retry restores the same cloned aggregate while retaining only the separately
  specified provenance identities;
- rejection drops the candidate aggregate;
- checkpoint and generation fork clone aggregate references;
- shipout borrows or moves its page root until the detached artifact is
  verified, then drops operation-local references.

No transition scans a graph to establish liveness, promotes a coordinate,
maintains a history registry, or records a pin. Serialization and TeX82 memory
accounting may traverse a graph already owned by an explicit `NodeListRef`;
that traversal produces detached data or diagnostics and has no lifetime role.

## Nested lists

Public `Node` fields such as box children, discretionary parts, leaders, math
choices, math lists, fractions, insertions, adjustments, and replay boxes are
`NodeListRef`. Compact `NodeRef` projections expose `NodeListId` only while the
enclosing payload is borrowed. Algorithms that descend through compact nodes
must resolve each coordinate through that same owner.

## Semantic identity

Node-list identity is allocation independent. It hashes semantic node content,
resolved child semantic identities, and referenced semantic values. Origins
remain diagnostic. Strong references and payload coordinates do not
participate in equality or checkpoint state hashes.

## Format and memo boundaries

Format capture and detached memo encoding begin from explicit structural
roots. Their bottom-up codecs assign dense keys to lists, erase process-local
payload coordinates, and validate children before parents during import.
Import reconstructs `NodeListRef` owners bottom-up and publishes the root only
after the complete graph validates.

Frozen Env box cells store a packed coordinate for TeX-compatible raw-word
projection and an aligned `NodeListRef` as the actual lifetime owner. The raw
word never authorizes lookup on its own.

## Accounting

Logical and retained payload bytes are reported by the structural owner.
Process-wide peak instrumentation observes allocations without retaining them.
There is deliberately no all-live node registry: aggregate memory is accounted
from the roots already in scope, and a derived accounting cache is discarded
when a box-root handoff cannot be updated without adding lifetime metadata.
