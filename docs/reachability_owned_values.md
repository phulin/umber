# Reachability-owned token, macro, and glue values

Status: architecture contract for Beads issue `umber2-3v8z.3` and its
children.

This document defines the immutable-value ownership boundary for token lists,
macro definitions, and glue specifications. It complements the live-state
identity contract in [Core Engine State](core_state.md) and the private
revision lifecycle in [Private revision allocation domains](patch_allocation_domains.md).

## Invariant

Persistent value memory is owned by a current semantic root, live restoration
authority, an explicit loaded-format base, or a detached published value. A
store lookup structure is never an owner:

```text
typed strong root -> immutable exact-content object
                         ^
                         |
                  weak or evictable index
```

Each object contains its complete immutable semantic payload and its cached
versioned semantic identity. Hashes select candidates only. Reuse requires an
exact structural comparison, including symbol semantic atoms for token lists
and recursively referenced exact objects for macro bodies. A collision may
cost lookup work but cannot alias two values.

Runtime slots, generations, capacities, allocation-domain coordinates, weak
index membership, and observation operands are physical metadata. They do not
enter live identity, format identity, memo identity, or exact equality.

There is no copying compactor, reachability sweep, accepted-generation
registry, or after-the-fact graph traversal. Every strong edge is installed or
removed at the typed mutation boundary that already knows both the owner and
the referenced value.

## Audited owners and restoration edges

The pre-migration stores own dense successful-history vectors. Aggregate
snapshots retain watermarks into those vectors, so a successful redefinition
cannot be reclaimed even after its Env cell, save-stack entry, page, node, and
checkpoint disappear. The following typed owners replace that accidental
authority.

| Value              | Current strong owners                                                                                                          | Restoration or transfer edges                                                                                  |
| ------------------ | ------------------------------------------------------------------------------------------------------------------------------ | -------------------------------------------------------------------------------------------------------------- |
| Token list         | Env token registers and parameters; macro bodies; command stored input and active macro calls; nodes and page marks; PDF state | Env undo entries; command summaries; Universe snapshots; generation forks; format bases; memo imports; patches |
| Macro definition   | Env meaning cells and active macro calls                                                                                       | Env undo entries; command summaries; Universe snapshots; generation forks; format bases; patches               |
| Glue specification | Env glue registers and parameters; compact and owned nodes; page `last_glue` and insertion state                               | Env undo entries; Universe and page snapshots; generation forks; format bases; memo imports; patches           |

The audit includes these less obvious edges:

- an equal local assignment may still create an Env undo owner;
- a later global assignment may retain, refile, or supersede a local undo;
- group exit and rollback can exchange the current and saved owners even when
  their raw words compare equal;
- a command input level can outlive redefinition of the macro whose replacement
  text it is delivering;
- page marks, insertion split glue, leaders, whatsits, PDF actions, PDF object
  data, document fragments, annotations, and outlines own values without an
  Env cell;
- a checkpoint owns its Env journal, input summary, page/PDF roots, command
  summary, mode/node roots, and loaded-format base as one closure;
- generation fork retargets runtime coordinates while sharing immutable
  payloads; and
- detached format and memo DTOs own semantic data rather than live runtime
  handles.

Transient borrowed reads are not roots. A builder owns unfinished content;
freezing transfers one strong reference to its typed destination or returns an
owned value that must be consumed by a destination. No API may return a bare
new runtime coordinate whose payload is kept alive only by the store.

## Object and slot representation

Each family has a private immutable payload behind shared ownership. An owning
reference pairs that payload with a timeline-local opaque coordinate used by
compact Env and node encodings. Related Universe forks share the payload but
mint or resolve coordinates through their own slot tables.

The dynamic slot table contains weak payload references and generation-safe
coordinate metadata. Dead slots are reusable without moving a live payload.
Reuse mints a fresh generation so a stale coordinate cannot alias later
content. Dead suffixes and reusable-slot metadata are trimmed or recycled at
ordinary allocation boundaries; capacity must plateau with the live and
configured cache high-water size.

The content index is either weak or explicitly bounded and evictable. Buckets
may disappear at any time. Lookup performs these steps:

1. compute the versioned semantic candidate key;
2. upgrade live weak candidates;
3. compare exact content, recursively where required;
4. reuse an exactly equal object or allocate a new immutable object; and
5. install only non-owning acceleration entries.

The canonical empty token list and zero glue may use immortal built-in values.
A loaded format owns an immutable frozen base explicitly. Later values use the
same exact lookup and reusable dynamic slots; the frozen base is not copied
into a successful-history overlay.

## Owner mutation contract

Env current cells and undo records store strong value references in parallel
with their compact semantic words. The write barrier changes both as one
operation. Journal coalescing, global supersession, group compaction, group
exit, rollback, and raw format-overlay installation must move or clone the
corresponding strong owner at the same point that they move or clone the raw
word. A receipt without the ownership disposition is insufficient.

Immutable nodes structurally own every token-list and glue value named by
their compact rows or sidecars. Node freeze validates and captures those
references before publication. Arena rollback drops the exact allocated
suffix and its references; survivor promotion shares the immutable references
with the promoted payload. This contract does not otherwise implement the node
promotion work tracked by `umber2-3v8z.6`.

Page, PDF, input, command, and mode structures store owning references in their
persistent copy-on-write roots. Replacing one field releases exactly that
field's old reference. Snapshot cloning shares these roots. Restoration swaps
the retained root; it does not reconstruct ownership by walking raw handles.

Macro body identity is distinct from both binding and provenance:

- the immutable body owns flags, parameter structure, parameter tokens, and
  replacement tokens;
- an Env meaning is the current binding of a control sequence to that body;
  Env undo records own older bindings; and
- definition and token-origin provenance is optional diagnostic metadata. It
  is excluded from semantic identity and cannot keep a semantic object alive
  merely through a lookup side table.

## Private revision lifecycle

Creating a new immutable value during private execution allocates its payload
in the revision's `PatchAllocationDomain` under the active aggregate operation
mark. The typed destination may also hold a strong reference inside the private
Universe. The domain remains the revision-level owner and records the exact
logical charge.

Ordinary failure and `NeedResource` rollback first restore typed destination
roots, then truncate the failed operation suffix. Earlier successful private
objects remain once. Rejecting the candidate or prepared transaction drops
both its roots and the complete domain.

Acceptance obtains deterministic typed roots from the owners that already
know them. The domain validates those roots and transfers only their distinct
objects. Structural children are owned by the selected objects themselves.
Unselected objects disappear with the domain. The accepted Universe retains
no domain, and acceptance performs no store walk or graph discovery.

## Identity, formats, and detached values

Canonical live-state hashing reads semantic identities through owning roots.
It never hashes physical coordinates or asks an index whether a value is live.
Equal reachable values therefore hash equally across dead allocation histories
and slot reuse; distinct exact content remains distinct even under a candidate
hash collision.

Format serialization assigns dense DTO indices from the explicit serialized
root closure and emits exact content once per DTO table. Format loading
validates complete content and child indices before publishing the frozen base.
Future append, redefinition, group restoration, and checkpoint rollback use
the ordinary object path above. A format must not serialize dead dynamic slots
or weak-index membership.

Memo and command-continuation detachment remain semantic-data operations. On
materialization they intern exact content and immediately install the returned
owning references in the destination roots. Their detached maps are not live
store authorities and do not justify retaining unrelated runtime values.

## Verification obligations

Each migrated family must have focused controls proving:

- deliberate candidate-hash collisions still compare exact content;
- every owner class remains readable while it is the sole root;
- removing the final root makes the weak payload and reusable slot collectible;
- equal local assignments, global supersession, open-group undo, group exit,
  rollback, and retained checkpoints preserve exact owners;
- source and loaded-format construction, future append, memo import, and
  generation fork preserve exact content;
- failure and `NeedResource` drop only the failed operation suffix, rejection
  drops all private work, and acceptance retains only explicit roots; and
- thousands of distinct redefinitions with bounded live groups/checkpoints
  plateau in live objects, slot metadata, and index capacity independently of
  command count.

An all-roots-live negative control must grow by the exact number and logical
bytes of intentionally retained values. This distinguishes genuine reclamation
from a test that merely stopped allocating.

## Migration order

`umber2-3v8z.3` is split into clean representation closures:

1. `umber2-3v8z.3.1` introduces the shared weak-slot substrate and migrates
   token lists across every audited owner;
2. `umber2-3v8z.3.2` builds macro bodies and bindings on reachability-owned
   token lists; and
3. `umber2-3v8z.3.3` migrates glue values using the same substrate.

The parent closes only after all three children and their combined plateau
controls pass.
