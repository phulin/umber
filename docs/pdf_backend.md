# Deterministic PDF backend

Status: phased implementation contract for the pdfTeX 1.40.27 backend.

## Boundary and invariants

PDF output is a downstream consumer of committed shipout artifacts, parallel
to DVI rather than a replacement for it. `tex-exec` continues to publish the
canonical page artifact and DVI page plan before the commit barrier. pdfTeX
mode additionally records PDF document state; TeX, e-TeX, and LaTeX-DVI modes
do not allocate or hash that state. Existing DVI artifact bytes and final DVI
assembly are unchanged.

The backend has three ownership layers:

1. An engine-owned PDF ledger holds the next indirect-object identity,
   explicitly created objects, and committed page/resource records. Its
   mutation goes through the `Universe` barrier and participates in groups
   where pdfTeX requires grouping, snapshots, rollback, semantic hashes, and
   the documented format-image policy.
2. Finalization detaches the ledger into a validated `tex-out` PDF document.
   The detached graph contains no `Universe`, node, font, or store handles.
3. A stateless `tex-out` adapter feeds the validated graph to the
   [`pdf_writer`](https://crates.io/crates/pdf-writer) crate, which assigns byte
   offsets and writes the header, indirect objects, page tree, catalog,
   cross-reference table, and trailer. The crate is the required PDF byte
   serialization implementation; Umber does not maintain a parallel handwritten
   PDF writer. The adapter does not mutate engine state and never publishes a
   partial file.

An indirect `PdfObjectId` is a nonzero 32-bit number. Allocation is monotonic
within one engine timeline. A snapshot records the next identity and ledger
roots; rollback removes allocations and mutations in the discarded suffix,
so replay receives the same identities. An object number is not a content
hash and is never reused on a live descendant timeline.

## Detached structural model

The detached model represents PDF values without text formatting ambiguity:
null, booleans, signed integers, normalized fixed-point numbers, byte names,
byte strings, arrays, dictionaries, indirect references, and streams.
Dictionaries are key-ordered. Stream length and structural trailer fields are
writer-owned, so callers cannot inject conflicting values.

An unvalidated document accepts indirect objects in arbitrary input order.
Validation sorts them by identity and rejects duplicate identities, dangling
references, an absent or non-dictionary catalog, conflicting writer-owned
keys, excessive depth/count/stream bytes, and page/resource graph violations.
Only the validated capability can be serialized. Its semantic identity hashes
a versioned canonical structural encoding, not eventual PDF spelling or byte
offsets. Equal graphs therefore have equal identities even if objects or
dictionary entries were supplied in different order.

Pages and resources use ordinary indirect objects with explicit identities.
The catalog points to one page-tree root; the tree owns an ordered `Kids`
array, and each page has one parent, media box, resources dictionary or
dictionary reference, and one or more content-stream references. Shared font,
image, form, and graphics-state resources are referenced rather than copied.
Future primitive issues may add object kinds without adding a second PDF
state store.

## Canonical serialization with `pdf_writer`

The initial adapter emits an uncompressed, deterministic PDF through
`pdf_writer`. It visits objects in ascending Umber identity order,
dictionaries in bytewise name order, and values in canonical graph order.
`pdf_writer` owns PDF token spelling, escaping, stream lengths, byte offsets,
the cross-reference table, trailer size, and final framing. Umber owns the
visit order, normalized fixed-point inputs, page/resource structure, and the
mapping from `PdfObjectId` to the crate's indirect-reference type.
Compression and object streams are later policies over the same structural
graph; they must not change semantic identity.

The selected crate version is pinned in the workspace lockfile and upgraded
only with deterministic-byte and fixture review. Final bytes include no wall
clock, random identifier, host path, hash-map
iteration order, or allocation address. Failure builds into a private buffer
and returns a typed error without publishing a prefix.

## Parity oracle

Committed minimal fixtures are regenerated only through
`scripts/regen-fixtures.sh`. The reference producer is pinned pdfTeX 1.40.27
with the repository's deterministic clock policy. Structural comparison
parses both files, removes byte-layout-only fields (offsets, xref spelling,
compression containers, and permitted volatile metadata), resolves indirect
references, and compares the normalized catalog/page/resource/content graph.
It must not discard drawing operators, geometry, font selection, resource
names, page order, or stream bytes after decompression.

Rendering comparison rasterizes corresponding pages with one pinned renderer
and fixed resolution/color settings, then compares dimensions and pixels. Any
tolerance must be explicit and limited to documented antialiasing edges; a
structural mismatch cannot be blessed by a visually similar page. Tests must
also run the complete committed DVI corpus byte-for-byte.

## Delivery gates

1. Detached model, validation, canonical semantic identity, and this design.
2. Valid deterministic PDF serialization for a minimal page.
3. pdfTeX shipout integration and checkpointed engine ledger.
4. Normalized pdfTeX structure fixtures, rendered-page fixtures, and the full
   DVI regression gate.

The parent backend issue is complete only when all four gates pass.
