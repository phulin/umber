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
typed sidecars, semantic spans, and a sorted, deduplicated set of strong
`NodeListRef` owners for the payloads named directly by those rows. Each child
payload owns its own direct children in turn. Cloning the reference clones
structural ownership; dropping the final root clone releases that closure when
no independently held child reference remains. Final-owner release drains that
closure with an explicit payload stack. A shared child stops the drain at that
edge and remains owned by its independent reference, while uniquely owned
descendants are reclaimed iteratively in work proportional to the released
closure rather than through Rust's recursive field destruction.

`NodeListId` is not an owner. It is a private compact coordinate used while a
`NodeListRef` is borrowed and in packed node sidecars. Detached codecs use a
separate `FormatListKey` vocabulary whose canonical form is the dense
bottom-up list ordinal plus its validated node count. Resolving a runtime child
coordinate requires the enclosing `NodeListRef`, which resolves its own spans
or delegates to the one direct child owner with the coordinate's payload root.
It does not scan descendants or consult a registry, and a coordinate cannot
upgrade or rediscover a dropped payload.

The canonical empty list is also a `NodeListRef`. Empty child coordinates
resolve without consulting global state. Detachment normalizes every
zero-length compact projection to that one empty DTO row before dense-key
assignment, so an enclosing payload's private zero span cannot leak into
format or memo bytes.

## Construction

`NodeListBuilder` is the sole mutable native-node builder. Ordinary mode
construction and packed command episodes append `Node` values directly to it;
there is no persistent per-command node vector or packed-only page-node
builder. General and mixed material uses native rows. Character/kern-only
episodes use an eight-byte inline row in the same builder and promote in place
if another node family arrives. Freezing performs these steps atomically:

1. collect and deduplicate direct child owners, materializing the reachability
   sidecar once;
2. validate non-node handles and direct child ownership;
3. compute allocation-independent semantic identity;
4. move-encode the root rows into one new immutable compact payload and attach
   the sorted distinct direct child-payload owners;
5. return one `NodeListRef` for the root span.

The newly owned root rows are move-encoded directly into their final payload
once. Preexisting immutable child payloads retain their existing coordinates
and allocation boundaries; freeze copies neither their compact words nor their
transitive descendants.

Validation failure publishes nothing. A weak, bounded candidate index may
reuse an exactly equal live payload, but weak entries neither retain payloads
nor recover dead ones.

Mode lists retain TeX's distinct semantic and physical diagnostic projections
as two uses of this same builder type. Their projection boundary/allocator
lineage vectors are nonsemantic sidecars, not a second node authority. Page,
shipout, named checkpoint, effect, observer/diagnostic, format,
state-identity/incremental, and terminal barriers cache immutable semantic and
physical `NodeListRef` sidecars. The next mutation invalidates only the
affected cache. Main-control rollback truncates the mutable builders through
its existing mode-journal marks and drops rejected frozen roots; it creates no
node-specific transaction or store.

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
must resolve each coordinate through that same owner, then use the returned
child owner for the next level. A root owner deliberately cannot resolve an
arbitrary grandchild coordinate that is absent from its direct owner set.
Iterative traversals therefore carry the current `NodeListRef` in every stack
frame rather than retaining one root owner beside a stack of bare cursors.

## Semantic identity

Node-list identity is allocation independent. It hashes semantic node content,
resolved child semantic identities, and referenced semantic values. Origins
remain diagnostic. Strong references and payload coordinates do not
participate in equality or checkpoint state hashes. The sorted owner set is a
lifetime and lookup structure only; its allocation order does not enter format
or memo bytes.

## Format and memo boundaries

Format capture and detached memo encoding begin from explicit structural
roots. Their bottom-up codecs assign dense keys to lists, erase process-local
payload coordinates, and validate children before parents during import.
Memo import first validates canonical key order, every detached content
reference, and every recomputed list semantic identity in private scratch
stores. Destination materialization then reconstructs `NodeListRef` owners
bottom-up and returns the root only after the complete graph validates. Frozen
format load validates the complete fresh store tuple before publishing it.

Frozen Env box cells store a packed coordinate for TeX-compatible raw-word
projection and an aligned `NodeListRef` as the actual lifetime owner. The raw
word never authorizes lookup on its own. A local assignment of the same owner
at a newly entered group depth still journals that owner: TeX82 §283 must
restore it at the save position. Only a repeated assignment already owned by
the current depth is an unchanged write.

## Accounting

Logical and retained payload bytes report the storage, semantic spans, and
direct-owner slots allocated by that payload, without recursively charging a
shared child payload again.
Process-wide peak instrumentation observes allocations without retaining them.
There is deliberately no all-live node registry: aggregate memory is accounted
from the roots already in scope. Its derived accounting cache survives
ordinary executor-operation boundaries, applies meaning and token-list Env
deltas at their existing write barriers, and refreshes the glue watermark on
the next observation. The cache is discarded when a box-root handoff cannot be
updated without adding lifetime metadata, including a box restoration during
aggregate rollback. Non-box rollback applies its root receipts before rejected
immutable-store suffixes are truncated.
The optional node candidate index reports its weak-entry count and retained
capacity to testing censuses only. Bounded-live controls require both values to
plateau; all-live controls sum exact logical and allocator-retained bytes over
the intentionally retained distinct payload references.

TeX82 memory reporting remains a projection of physical TeX allocation
lifetimes, not host payload coordinates. In particular, a detached diagnostic
branch contributes its complete historical high-memory subtree even when weak
interning gives it the same immutable host payload as a live semantic branch.
Allocator-overlap metadata removes only cells whose TeX lifetimes actually
overlap; equal semantic content never suppresses an independent allocation.

## Final raw-coordinate audit

Every production `NodeListId` site is one of these non-owning classes:

| Site                                                                  | Classification                                                                                                                                        |
| --------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------- |
| `node_arena::{storage,tables,view,copy,mutation,schema,owned}`        | Payload-internal coordinate or borrow-scoped traversal/copy frame under an enclosing `NodeListRef`.                                                   |
| `Env`, box-bank raw words, state hashing, and TeX82 memory projection | Parallel compact projection whose box slot or projection call borrows the structural owner.                                                           |
| node semantic composition and handle validation                       | Operation-local callback operand resolved through the builder or enclosing payload.                                                                   |
| shipout and alignment compact adapters                                | Borrow-scoped `BoxNode<NodeListId>` or `UnsetNode<NodeListId>` consumed before the page/form owner borrow ends.                                       |
| format and memo implementation                                        | Capture-local runtime key before canonical remapping, decode-local span while constructing one unpublished payload, or dense `FormatListKey` DTO key. |
| PDF `box_list()`                                                      | Read-only coordinate projection paired with the record's authoritative `box_list_ref()` owner.                                                        |

No page, mode, command, checkpoint, revision, PDF, DVI, HTML, or published
artifact value owns one of these coordinates. `tex-out` has no `tex-state`
dependency and its page, DVI, PDF, positioned, and HTML models retain semantic
data only.
