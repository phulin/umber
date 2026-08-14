# Compact node-word arena

Status: authoritative contract for the adopted compact node representation and
installed directly owned substrate. Nested and aggregate owner migration is
tracked separately and is not claimed here.

An immutable TeX node-list graph is an eight-byte word stream with kind-specific
sidecars behind `NodeListRef`. The ref directly owns one `Arc` payload plus an
exact validated span and semantic projection. A compact coordinate is a private,
borrow-scoped projection into that payload, never lifetime authority. Raw words,
indexes, coordinates, and native layout remain implementation details rather
than artifact formats.

## Core invariants

- A directly frozen graph has exactly one mutable construction authority:
  operation-local `NodeListBuilder`. Consuming freeze validates first and
  publishes one immutable `NodeListRef` only after the complete graph exists.
- `NodeListRef` is the sole strong owner in the installed direct substrate.
  Cloning shares exact immutable data; final drop releases the payload.
- The canonical empty list has one explicit immutable owner.
- Every compact storage field and sidecar length is inside the immutable
  payload. Legacy identity tables, survivor refcounts, and rollback marks
  remain inside `Universe`/`Stores` until their owning strata migrate.
- Legacy rollback truncates one aggregate node mark; it cannot restore only
  part of a word/sidecar tuple.
- A compact coordinate can resolve only while borrowing its matching
  `NodeListRef`; a weak projection cannot upgrade after final drop.
- Direct freeze copies registered child owners into one self-contained compact
  graph. Related owners may share that immutable payload.
- Semantic hashes traverse decoded logical nodes and referenced content, never
  raw tags, indexes, capacities, addresses, or allocation order.
- Downstream crates receive builders and decoded read-only views, never raw
  words, mutable columns, unchecked constructors, or sidecar indexes.

## `NodeWord` encoding

`NodeWord` is a private transparent `u64`; a compile-time assertion fixes its
size at eight bytes. Bits 63..59 are a five-bit tag and bits 58..0 are a
59-bit payload. Unused payload bits are zero and raw words are not serialized.

|    Tag | Kind            | Payload, low bits first                                |
| -----: | --------------- | ------------------------------------------------------ |
|      0 | char            | Unicode scalar 21, `FontId` 32                         |
|      1 | ligature        | char 8, left original 8, right original 8, `FontId` 32 |
|      2 | kern            | signed `Scaled` 32, `KernKind` 2                       |
|      3 | leaderless glue | `GlueId` 32, `GlueKind` 6                              |
|      4 | penalty         | signed `i32` 32                                        |
|      5 | math-on         | signed `Scaled` 32                                     |
|      6 | math-off        | signed `Scaled` 32                                     |
|      7 | math-style      | `MathStyle` 2                                          |
|      8 | nonscript       | zero                                                   |
|      9 | hlist           | box sidecar index 32                                   |
|     10 | vlist           | box sidecar index 32                                   |
|     11 | unset           | unset sidecar index 32                                 |
|     12 | rule            | rule sidecar index 32                                  |
|     13 | leader glue     | leader sidecar index 32                                |
|     14 | discretionary   | discretionary sidecar index 32                         |
|     15 | mark            | mark sidecar index 32                                  |
|     16 | insertion       | insertion sidecar index 32                             |
|     17 | whatsit         | whatsit sidecar index 32                               |
|     18 | math noad       | noad sidecar index 32                                  |
|     19 | fraction noad   | fraction sidecar index 32                              |
|     20 | math choice     | choice sidecar index 32                                |
|     21 | math list       | math-list sidecar index 32                             |
|     22 | adjust          | adjust sidecar index 32                                |
| 23..31 | reserved        | invalid until an in-memory migration assigns one       |

Sidecar indexes remain 32-bit even though the payload is wider. Constructors
validate Unicode scalars, TFM-byte ligature fields, signed bit preservation,
and exhaustive discriminant mapping. A glue with a leader always uses tag 13;
its sidecar owns the full leader payload. Capacity is checked before any word
or sidecar length changes.

## Owned refs and compact coordinates

`NodeListRef` contains an `Arc<NodeListPayload>`, the exact top-level span, and
its precomputed semantic identity. The payload contains `NodeStorage` and a
private sorted table of exact child spans and semantic identities. Resolving a
child checks the payload coordinate and exact span, then returns a `NodeList`
whose lifetime is bounded by the owner borrow. It does not consult
`SurvivorArena`, a registry, a root slot, or a pin.

The canonical empty owner is process-shared and consumes no node word. Empty
children use only its canonical projection; arbitrary zero-length coordinates
are rejected.

The optional candidate index retains at most 64 weak entries. Candidate
fingerprints are acceleration only: reuse also requires the full semantic
identity and an exact normalized graph comparison including diagnostic and
physical-only content. Clearing or evicting the index changes neither liveness
nor meaning. Ten thousand bounded-live replacements retain bounded weak
metadata; an all-live control grows by the exact graph count and logical bytes.

## Transitional `NodeListId`

`NodeListId` is still the compact sixteen-byte runtime coordinate used by
unmigrated node fields and aggregate owners. Epoch coordinates use the
common `(namespace, generation, slot)` identity; the arena maps the slot to a
compact `(start, len)` span. Lookup performs bounds and generation validation
before loading the span. Empty epoch lists use one immutable built-in identity.

Survivor handles retain a self-contained packed identity:

```text
survivor: 1 | root:20 | start:21 | len:22
```

Epoch spans support `u32` starts and lengths through `2^31-1`. Survivor spans
support roots through `2^20-2`, starts through `2^21-1`, and lengths through
`2^22-1`. The all-ones word is the canonical `None` encoding in the Env box
bank. Epoch handles never enter raw Env words; assignment promotes them first.

One legacy `NodeArenaMark` contains the identity-table and compact-storage watermarks.
Rollback validates them, truncates the identity suffix, advances the generation
before slot reuse, and then truncates words and every sidecar. Arena clones
preserve inherited tags and use a fresh namespace for new allocations.

Direct ownership does not expose or resolve a coordinate through a store table.
The current packed survivor root field uses a process-unique payload coordinate
only so pre-migration compact child rows retain their native layout. It is
non-authoritative, cannot upgrade a dead payload, and has no historical
generation table.

Handles are not serialized. Frozen formats use private logical node DTOs and
dense content keys, validate the complete graph, and mint fresh identities on
restore. Artifacts and hashes encode logical content rather than runtime ids.

## Sidecar storage

Each `NodeStorage` owns one word vector and all sidecars. Structure-of-arrays
columns are used where fields are independently scanned; columns advance in
lockstep. Boxes remain row-packed because consumers commonly decode and patch
complete `BoxNode` values.

The storage includes:

- one raw diagnostic-origin projection and one optional strong origin root
  aligned with every word, plus raw projections and strong roots aligned with
  every consumed character in a ligature;
- boxes, unsets, rules, leader glues, discretionaries, marks, insertions, and
  adjustments whose sidecar retains the pdfTeX pre-migration marker;
- detached whatsit payloads, including owned strings and bytes;
- noads, fractions, choices, and math lists; and
- child-list and shared-content handles required by each logical row.

Origin roots move and roll back with their projections, including compact
survivor promotion, but do not participate in node equality or semantic
identity. Profiling accounts for both the aligned root column and the ragged
ligature-root allocations. Small nested sum types may remain packed columns
when splitting them would increase size or branching.

## Publication and rollback

Capturing `NodeArenaMark` is O(1). Rollback validates every target length before
truncating all columns and the word stream as one private operation. No public
API can mark, truncate, append a raw word, append an isolated sidecar row, or
restore a subset.

Direct builder freeze is transactional with respect to logical state: validate
every ordinary handle and directly registered child owner, preflight capacity,
construct and patch the complete self-contained graph in unpublished local
storage, then return its owner. Validation failure publishes no payload and
changes no aggregate destination. Frozen words and sidecars never mutate.

The existing epoch path remains during the dependency-ordered owner migration.
Its successful operation still promotes referenced graphs before truncating
the epoch suffix; retry or rejection truncates without publication. This is a
transition path, not ownership authority for the new direct substrate.

## Legacy aggregate bridge

The pre-existing survivor bridge now stores the same `NodeListPayload` used by
`NodeListRef`, and its structural owner contains a `NodeListRef`. This lets the
substrate land without copying a second immutable representation. It does not
declare the remaining raw owners migrated: Env box current/undo, page, mode,
control, checkpoints, PDF forms, shipout, format installation, pins, and legacy
root refcounts are retired only by the later ownership children.

No new compatibility owner, lifetime sidecar, registry, pin, graph scan,
compactor, or successful-history table is introduced by the substrate.
Promotion continues to copy a mixed legacy epoch/survivor DAG into one compact
payload and rewrites every child coordinate before publication.

Live box registers and retained undo records own survivor references. Publishing
a box into nest or page state adds one aggregate root pin; one pin covers every
interior span. Snapshots and shipout scopes capture the pin-log length and drain
only their suffix on rollback or release. Group exit does not independently
truncate node pins. Format capture requires a quiescent empty runtime pin log.

Rollback-coupled engine records that retain a node list after its originating
allocation scope use a separate timeline pin log. In particular, a PDF form
owns the box removed from its register until aggregate rollback removes that
form. Box-build and shipout completion never drain timeline pins; snapshots
capture both pin-log lengths, and rollback releases the corresponding suffixes
before truncating survivor storage. This follows TeX.web §§1073--1086, where box
construction transfers a live box pointer through `box_end`, and pdftex.web
§1546, where `\pdfxform` clears the register but stores that pointer in the form
object for later recursive traversal (§§773--775).

At local legacy refcount zero with no direct ref, the old root slot is removed.
Its vectors enter the recycled pool only if `Arc::try_unwrap` proves that no
direct owner or related Universe shares the payload; otherwise teardown is an
O(1) shared-payload drop. This recycling is allocator policy, not liveness
authority.

## Access boundary

The node API exposes `NodeListRef`, consuming builders,
`NodeList<'a>`/`NodeIter<'a>` read-only views, decoded `NodeRef<'a>` accessors,
and a `NodeCursor<'a>` that presents the same logical view over owned
construction lists and compact epoch or survivor lists.
`PackedNode` is the shared dimension-bearing projection. Semantic child
traversal excludes diagnostic-only box children; physical traversal includes
them. `NodeListRef::child_nodes` validates an internal coordinate and returns a
view tied to the ref borrow, so direct child traversal needs no
`SurvivorOwner`. The API never exposes a raw word slice, sidecar slice/index,
unchecked decoder, raw handle constructor, or mutable storage.

All direct rewriting is builder-then-freeze and creates one self-contained
payload. The legacy path may still retain unchanged survivor-backed descendants
under an existing root pin until its owning stratum migrates. No algorithm
mutates a frozen word or sidecar row. Pure typesetting receives immutable views
and copied parameters; execution owns publication and box-register writes;
shipout lowers into detached artifacts and cannot retain a live view.

## Semantic hashing and width scans

One private logical schema records every node kind's fields, e-TeX category,
dimension behavior, and child-traversal shape. Hashing dispatches through
`NodeRef` and follows the same logical fields and referenced content as the
decoded node model. Sidecar indexes, raw handle bits, root ids, capacities,
recycling order, and addresses are excluded. Tests compare hashes across
rollback/reappend, promotion, release, different allocation orders, and
recycled-capacity reuse.

Loaded TFM metrics expose a dense byte-character width array. A scan may combine
a contiguous same-font run of inline character words after validating the font
once. Scalar, unrolled, or target-selected vector implementations must preserve
TeX's exact `Scaled` order and overflow behavior. Ligatures, missing characters,
modern non-byte glyphs, font changes, and non-character nodes end the run.

## Validation matrix

Validation covers every tag and reserved tag, signed extrema, Unicode and TFM
bounds, identity/namespace/generation liveness, null and capacity boundaries,
sidecar alignment, transactional freeze, exact collision separation, canonical
empty ownership, child-span resolution, clone/final-drop behavior, stale weak
projection rejection, bounded weak metadata, exact all-live growth, semantic
hash equivalence across physical coordinates, legacy rollback and survivor
recycling, compile-fail access checks, all typesetting kernels, width runs,
shipout, exact fixture/DVI parity, and logical/retained allocation budgets.
