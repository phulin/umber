# Shared box closures

Status: normative amendment to [Node-region ownership](node_region_ownership.md)
for explicit box copies, 2026-09-29. Where that document requires explicit
`\copy`, `\unhcopy`, and `\unvcopy` to duplicate the complete recursive
closure, or forbids a region from naming a list owned by another region, this
document takes precedence for the frozen regions defined here. Every other
rule of the ownership contract still applies.

## Motivation

LaTeX copies large boxes repeatedly and reads the copies without changing
them. In the pinned book workload, the shipout code unpacks the whole page
into a background wrapper with `\unvcopy`, and the mark code unpacks the
page body with `\unvcopy` only to `\vsplit` it for marks. Listings copies a
small frame box once per source line. The copies moved 28.5M nodes and cost
4--5% of all cycles, although almost every copied subtree was only read
before it died.

TeX82 §§204--206 give `copy_node_list` value semantics. They do not require
a physical copy of an immutable subtree, only that no later change to either
copy can reach the other. Umber's sealed node chunks are already immutable.
The only in-place mutation is the root-box dimension write of
[Durable box root mutation](durable_box_root_mutation.md), which never
reaches a child list.

## Model

A _frozen region_ is a durable node region that the node pool owns directly.
The pool never mutates a frozen region, appends to it, moves its chunks, or
truncates it. A frozen region retires only when no other region can name
any of its lists.

A _borrowed coordinate_ is a nonempty child-list coordinate stored in a
record owned by region R, where the list itself belongs to a frozen region F.
Only frozen regions may be borrowed from. A borrowed list is read-only for
every region except F, and F is read-only too.

### Freezing a register

The first explicit copy of durable register closure C freezes it:

1. The pool publishes a one-record region C' holding a copy of C's root
   record. Its annex is relocated as usual, and its child list stays in C.
2. C's region moves into the pool as frozen region F. C' replaces C in the
   same durable owner slot, keeping the same owner id, lineage, and semantic
   identity.
3. C' logs one share of F.

TeX cannot observe the split. Owner ids, journals, and scalar inverses name
the slot, not the region. A later root-dimension write edits C's root
record, whose annex is private. `\box`, `\unhbox`, `\unvbox`, `\vsplit`,
and history-preservation copies of C' see an ordinary exclusive region with
borrowed children.

### Shallow copies

An explicit copy of a register whose closure is frozen, or has just been
frozen, is shallow:

- `\copy n` copies C''s root record into the destination list and borrows
  its child list.
- `\unhcopy n` and `\unvcopy n` copy the top-level records of the root box's
  child list, which belongs to F, and borrow those records' children.

The destination region logs one share of F. The cost is O(top-level
records), independent of depth.

The general recursive copy walk (`CopyContext`) shares a box body when
every nonempty child list of the box is frozen. That body is already
borrowed, so copying it would be wasted work. The walk logs the share in
the destination, and the copied wrapper carries no body stamp, so no
interval move can claim the borrowed body. As a result, structural-copy
fallbacks, page-to-durable copies, and held-over evacuation of material with
borrowed children stay shallow at every borrowed boundary.

Only closures of at least eight live payload chunks freeze. A smaller box
copies faster than it splits.

The automatic output box 255 is page-owned and is still deep-copied.
`\shipout\copy` keeps its pinned in-place read.

## Lifetime

Each `NodeRegion` carries a _borrow log_: a duplicate-free set of
frozen-region references. Each log entry holds one share, counted in the
pool's frozen-region registry. A region's log is a superset of the frozen
regions its records name:

- A shallow copy or `CopyContext` share logs its frozen region in the
  destination as it publishes the borrowed coordinate.
- Every move of records from one region into another logs every entry of
  the source into the destination. This covers whole-region transfers,
  suffix moves (closure builds), interior-interval moves (unique takes and
  generated boxes), shared-prefix successors, and consumed-cut rewrites
  whose mapped children may keep borrowed coordinates.
- Rollback never removes entries. An over-long log only delays reclamation
  until the region retires.

The inheritance is deliberately conservative. A borrowed list adds no
dependency floor, so it never forces a structural copy, and no move needs
to prove which borrowed coordinates it carries.

Retiring any region (`NodePool::retire_region_in_place`) releases its
log's shares. When a frozen region's share count reaches zero, it retires
too, releasing its own log. The release uses an explicit worklist, not
recursion.

The page region is the one long-lived region that sheds material without
retiring: unique-successor adoption keeps its region and drops the consumed
prefix. Adoption therefore rebuilds the log exactly. It scans the adopted
suffix's records for frozen child lists, logs those, and then releases the
old entries. The scan runs only when the log is nonempty, and it costs one
pass over the held-over material. No checkpoint survives adoption, so no
rollback can restore a record that named a released entry.

Checkpoint candidates move page regions and durable owners, never the pool.
The registry therefore stays single-lineage. Accepting or rejecting a
candidate retires regions through the same choke point.

## Resolution

Readers resolve a list through its owning region. A frozen region has no
TeX owner, so the pool resolves it by arena identity: the chunk metadata of
the list head names the owning arena, and the registry maps that arena to
its frozen slot. Page-list resolution first admits a list against its
expected region, as today. Only on a foreign-arena failure does it consult
the registry, so unshared lists pay no lookup.

Operations that need ownership of a list (splicing, unique transfer,
consumed-box projection, `\vsplit` of a child list, discretionary splicing
in line breaking) first _materialize_ a borrowed list: they shallow-copy its
top-level records into the operating region and log the share. Admission
against the wrong region fails with an error, never with corruption. A
missing materialization site therefore surfaces as an admission failure
under test.

## Invariants

- Frozen regions and the chunks they own are immutable.
- A borrowed coordinate names a list owned by a live frozen region.
- Every region's borrow log covers every frozen region its records name.
- A frozen region with zero shares is retired immediately.
- Semantic list identities are unchanged: a shallow copy carries the same
  identity that a deep copy of the same closure would.
- Format dumps, state hashes, and shipout read through the resolver and
  never serialize a coordinate. A loaded format owns no frozen region.

## Measurement

Counters record frozen-region creation, shallow-copied records, borrowed
coordinates shared by `CopyContext`, materializations, and frozen
retirements. The pinned book workload must keep its aux, toc, out, and PDF
content identical. The gain is judged by paired instruction counts.
