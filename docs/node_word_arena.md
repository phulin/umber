# Compact node-word arena

Status: authoritative contract for the adopted compact node representation.

The arena stores immutable TeX node lists as an eight-byte word stream with
kind-specific sidecars. Runtime list identities are generation-tagged and
owner-validated; retained survivor graphs are immutable, self-contained roots.
Raw words, indexes, and handles are implementation details rather than artifact
formats.

## Core invariants

- Frozen lists are immutable and minted only through the aggregate state API.
- Every semantic or ownership field, sidecar length, identity table, survivor
  refcount, and rollback mark is inside `Universe`/`Stores` ownership.
- Rollback truncates one aggregate node mark; it cannot restore only part of a
  word/sidecar tuple.
- Discarded epoch identities and survivor root identities never revive.
- Survivor promotion copies a graph bottom-up into one self-contained root;
  related Universes may share its immutable payload.
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

## Generation-tagged `NodeListId`

`NodeListId` is a private sixteen-byte runtime identity. Epoch handles use the
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

One `NodeArenaMark` contains the identity-table and compact-storage watermarks.
Rollback validates them, truncates the identity suffix, advances the generation
before slot reuse, and then truncates words and every sidecar. Arena clones
preserve inherited tags and use a fresh namespace for new allocations.

Survivor root keys are monotonic and never reused. Storage buffers may recycle
after the last shared payload owner disappears, but recycled capacity cannot
affect liveness, meaning, hashes, or output. Exhaustion is explicit and never
aliases the null encoding.

Handles are not serialized. Frozen formats use private logical node DTOs and
dense content keys, validate the complete graph, and mint fresh identities on
restore. Artifacts and hashes encode logical content rather than runtime ids.

## Sidecar storage

Each `NodeStorage` owns one word vector and all sidecars. Structure-of-arrays
columns are used where fields are independently scanned; columns advance in
lockstep. Boxes remain row-packed because consumers commonly decode and patch
complete `BoxNode` values.

The storage includes:

- one diagnostic origin aligned with every word, plus consumed-character
  origins for ligatures;
- boxes, unsets, rules, leader glues, discretionaries, marks, insertions, and
  adjusts;
- detached whatsit payloads, including owned strings and bytes;
- noads, fractions, choices, and math lists; and
- child-list and shared-content handles required by each logical row.

Origins move and roll back with storage but do not participate in node equality
or semantic identity. Small nested sum types may remain packed columns when
splitting them would increase size or branching.

## Publication and rollback

Capturing `NodeArenaMark` is O(1). Rollback validates every target length before
truncating all columns and the word stream as one private operation. No public
API can mark, truncate, append a raw word, append an isolated sidecar row, or
restore a subset.

Builder finish is transactional with respect to logical state: validate child
handles and capacity, reserve all required columns, append sidecar rows, and
publish words last. Bottom-up construction requires epoch children to end
before their parent span. Retained vector capacity after rollback is allocator
state only and is reported separately from live logical bytes.

## Survivor roots and sharing

Every survivor root payload owns a complete immutable `NodeStorage`. The local
root slot holds that payload through `Arc`, its refcount, and an optional
diagnostic-origin overlay. Promotion iteratively decodes a mixed epoch/survivor
DAG, memoizes source spans, appends logical nodes to the destination, and
rewrites child handles to the new root. No sidecar index crosses a root.

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

At local refcount zero the root slot is removed. Its vectors enter the recycled
pool only if `Arc::try_unwrap` proves that no related Universe still shares the
payload; otherwise teardown is an O(1) shared-payload drop.

Accepted paragraph history mounts these shared roots directly. Its handle owns
the payload and deduplicated glue closure. A restarted Universe validates the
handle, installs it under an ordinary rollback pin, restores compatible glue
resources, and overlays current-revision character origins without changing
semantic words or ids. Unsupported handle-bearing forms are rejected before
mutation.

## Access boundary

The node API exposes builders, `NodeList<'a>`/`NodeIter<'a>` read-only views,
decoded `NodeRef<'a>` accessors, and narrow immutable traits for pure typesetting
kernels. It never exposes a raw word slice, sidecar slice/index, unchecked
decoder, raw handle constructor, or mutable storage.

All rewriting is builder-then-freeze. A changed top-level list may retain
unchanged survivor-backed descendants under a root pin, but no algorithm mutates
a frozen word or sidecar row. Pure typesetting receives immutable views and
copied parameters; execution owns publication and box-register writes; shipout
lowers into detached artifacts and cannot retain a live view.

## Semantic hashing and width scans

Hashing dispatches through `NodeRef` and follows the same logical fields and
referenced content as the decoded node model. Sidecar indexes, raw handle bits,
root ids, capacities, recycling order, and addresses are excluded. Tests compare
hashes across rollback/reappend, promotion, release, different allocation
orders, and recycled-capacity reuse.

Loaded TFM metrics expose a dense byte-character width array. A scan may combine
a contiguous same-font run of inline character words after validating the font
once. Scalar, unrolled, or target-selected vector implementations must preserve
TeX's exact `Scaled` order and overflow behavior. Ligatures, missing characters,
modern non-byte glyphs, font changes, and non-character nodes end the run.

## Validation matrix

Validation covers every tag and reserved tag, signed extrema, Unicode and TFM
bounds, identity/namespace/generation liveness, null and capacity boundaries,
sidecar alignment, bottom-up graphs, rollback, survivor sharing and recycling,
compile-fail access checks, semantic hash equivalence, all typesetting kernels,
width runs, shipout, exact fixture/DVI parity, and logical/retained allocation
budgets.
