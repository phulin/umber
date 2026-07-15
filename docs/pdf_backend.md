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

The implemented engine ledger reserves object 1 for the catalog and object 2
for the page-tree root. Each successful pdfTeX-mode shipout atomically reserves
the next three identities for its resource dictionary, content stream, and
page dictionary and records the committed artifact hash beside them. The
append occurs only after artifact storage and effect commit succeed. Scoped
shipout failure, ordinary snapshot rollback, and retained-generation rollback
therefore remove the entire suffix and replay the same identities and semantic
hash. A format may be dumped with an enabled but empty ledger; any committed
PDF page makes the format ineligible, and the ledger itself is omitted from
the format image.

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

The implemented adapter pins `pdf-writer` 0.15.0 and emits deterministic PDF
through that crate. It visits objects in ascending Umber identity order,
ordinary dictionaries in bytewise name order, and values in canonical graph
order. The typed catalog writer owns its required `/Type /Catalog` entry; the
remaining catalog entries retain model order.
`pdf_writer` owns PDF token spelling, escaping, stream lengths, byte offsets,
the cross-reference table, trailer size, and final framing. Umber owns the
visit order, normalized fixed-point inputs, page/resource structure, and the
mapping from `PdfObjectId` to the crate's indirect-reference type.
Compact, uncompressed output is the default. Options can request
`pdf_writer`'s pretty layout or deterministic zlib/DEFLATE stream compression
at levels 0--9; the adapter supplies compressed bytes and declares
`/FlateDecode` through the crate's stream API. Automatic compression rejects
streams that already declare `/Filter` or `/DecodeParms`. Object streams are a
PDF 1.5-or-newer adapter policy over the same structural graph. The pinned local
`pdf-writer` fork exposes an object-stream builder that serializes eligible
non-stream values through `Obj`, registers their container and index, and emits
matching type-2 entries from `finish_with_xref_stream`. Positive
`\pdfobjcompresslevel` values 1--3 enable that path; stream objects remain
ordinary type-1 entries. When stream compression is enabled, ordinary,
object, and cross-reference streams all declare deterministic Flate encoding
through `pdf_writer`. None of these byte policies change semantic identity.

The selected 0.15.0 source fork is path-pinned in the workspace manifest and
lockfile, retains both upstream licenses, and records its crates.io checksum
and modifications in `vendor/pdf-writer/PROVENANCE.md`. It is upgraded only
with deterministic-byte and fixture review. `pdf_writer`
supports positive signed-32-bit references and signed-32-bit integers; the
adapter preflights the broader detached types and returns typed range errors
instead of entering a panicking crate path. All output remains private until
`Pdf::finish` succeeds. A hermetic test parses both compressed and
uncompressed results with an independently pinned `lopdf` development
dependency. Final bytes include no wall
clock, random identifier, host path, hash-map
iteration order, or allocation address. Failure builds into a private buffer
and returns a typed error without publishing a prefix.

The Umber driver lowers committed positioned rule events into `pdf_writer`
content operations, builds the detached catalog/page/resource graph, and
serializes only after validation. `umber run --pdftex --pdf <path>` publishes
the resulting private buffer through the same effect-before-driver
finalization barrier as DVI and HTML. The current integration deliberately
returns typed errors for text and specials until their resource-owning
primitive slices are implemented; a minimal rule-only page is complete.

pdfTeX mode freezes `\pdfoutput`, the PDF version, stream/object compression,
and decimal precision at the first committed shipout. Invalid major/minor
versions recover to 1.4 with the pinned diagnostics; compression and precision
use pdfTeX's bounded first-write values. Later output-mode or version changes
are fatal setup errors. `\pdfdecimaldigits` controls page-box and rule-number
rounding, and DVI plans remain byte-identical whether PDF mode is selected or
not.

## Parity oracle

`tests/corpus/tex_exec/pdf_output_policy` is regenerated with pinned pdfTeX
1.40.27 in INITEX mode. It commits the output-control defaults, TeX grouping,
first-write range recovery, warning text, and recovered version. Hermetic
Umber tests pair that fixture with alias-cell, fatal setup, version header,
object-stream/type-2-xref, decimal rounding, and DVI/PDF switching assertions.

Committed `tests/corpus/pdf` minimal fixtures are regenerated only through
`scripts/regen-fixtures.sh --area pdf` or its `--case pdf/<case>` form. The
reference producer is pinned pdfTeX 1.40.27 with the repository's deterministic
clock policy. Structural comparison
parses both files, removes byte-layout-only fields (offsets, xref spelling,
compression containers, and permitted volatile metadata), resolves indirect
references, and compares the normalized catalog/page/resource/content graph.
It must not discard drawing operators, geometry, font selection, resource
names, page order, or stream bytes after decompression.

Rendering uses Poppler `pdftoppm` 25.08.0 at 72 dpi in grayscale mode. The
regenerator requires exact PGM equality, with no pixel or antialiasing
tolerance, and commits the raster plus an attestation containing the renderer
arguments and SHA-256 identities of both input PDFs and the equal raster.
Ordinary cargo tests remain hermetic: they reproduce the committed Umber PDF
bytes, repeat structural normalization against the committed pdfTeX PDF, and
verify the raster attestation chain without launching pdfTeX or Poppler. A
structural mismatch cannot be blessed by a visually similar page. Tests also
run the complete committed DVI corpus byte-for-byte.

## Delivery gates

1. Detached model, validation, canonical semantic identity, and this design.
   **Done.**
2. Valid deterministic PDF serialization for a minimal page. **Done.**
3. pdfTeX shipout integration and checkpointed engine ledger. **Done.**
4. Normalized pdfTeX structure fixtures, rendered-page fixtures, and the full
   DVI regression gate. **Done.**

The parent backend issue is complete only when all four gates pass.
