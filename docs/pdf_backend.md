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

The implemented engine ledger begins at object 1, so the first user object or
font enquiry observes the same identity as pdfTeX. An ordinary successful
pdfTeX-mode shipout atomically reserves its resource dictionary, page
dictionary, and content stream in that order. A page identity reserved earlier
by an action is reused while shipout allocates only its resource and content
objects. Page-tree, optional Names, catalog, and optional Info dictionaries are
allocated idempotently from the same ledger during finalization, after page
objects, in pdfTeX order. The page append occurs only after artifact storage
and effect commit succeed. Scoped
shipout failure, ordinary snapshot rollback, and retained-generation rollback
therefore remove the entire suffix and replay the same identities and semantic
hash. A format may be dumped with an enabled but empty ledger; any committed
PDF page makes the format ineligible, and the ledger itself is omitted from
the format image.

Each page receipt also captures the page-local `\pdfhorigin`, `\pdfvorigin`,
`\pdfpagewidth`, `\pdfpageheight`, `\pdfpageattr`, and
`\pdfpageresources` values. Finalization reads `\pdfpagesattr` from the final
document state, matching pdfTeX's page-tree scope, while the PK bitmap mode is
fixed by the first PDF page. Explicit nonzero page dimensions win; zero
dimensions fall back to the shipped box extent plus twice the corresponding
origin and TeX offset. Content coordinates consume the captured origins.

## Detached structural model

The detached model represents PDF values without text formatting ambiguity:
null, booleans, signed integers, normalized fixed-point numbers, byte names,
byte strings, arrays, dictionaries, indirect references, and streams.
Dictionaries are key-ordered. Stream length and structural trailer fields are
writer-owned, so callers cannot inject conflicting values.
The dictionary model additionally carries one semantically hashed verbatim
entry fragment for pdfTeX extension attributes. These bytes are intentionally
not parsed: pdfTeX copies token-list attributes directly into dictionaries.
Typed required entries remain validated, except that a raw `/MediaBox`
suppresses and satisfies the automatically generated page box just as it does
in pdfTeX.

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

Catalog open actions are distinct from verbatim catalog extension entries.
The engine retains a typed shared action specification and reserves the action
plus any internal destination, structure, or forward-page identity at scan
time. A forward page later consumes its preassigned page-dictionary identity
while allocating only its resource and content objects. These reservations are
part of snapshots and semantic hashes, so rollback and replay reproduce both
the action and the nonsequential page-object ordering used by pdfTeX.

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
PDF 1.5-or-newer adapter policy over the same structural graph. The pinned
`pdf-writer` fork exposes an object-stream builder that serializes eligible
non-stream values through `Obj`, registers their container and index, and emits
matching type-2 entries from `finish_with_xref_stream`. Positive
`\pdfobjcompresslevel` values 1--3 enable that path; stream objects remain
ordinary type-1 entries. When stream compression is enabled, ordinary,
object, and cross-reference streams all declare deterministic Flate encoding
through `pdf_writer`. None of these byte policies change semantic identity.
Type3 bitmap glyph streams also remain crate-owned: the fork's content
builder accepts typed width, height, image-mask, bit depth, decode array, and
payload inputs and writes the complete inline-image operation. Umber never
handwrites `BI`, `ID`, or `EI` framing.

The selected 0.15.0 source fork is `phulin/pdf-writer` commit
`030c3b1ad0e528b13ee3e6ca4605c91fbeaa3d91`, revision-pinned through
the direct workspace dependency and lockfile. It descends
directly from upstream 0.15.0 source commit
`639214e1745f2b1ff29ad0621da151807118d7bc`, whose crates.io package checksum
is `f5e456864a7a304047bff84977dc6fb162bd956475d40ba50b2dcecaada7f753`, and
retains both upstream licenses. It is upgraded only with deterministic-byte
and fixture review. Its small extension adds typed
object-stream construction and compressed-object registration so all PDF and
xref bytes remain owned by `pdf_writer`. It also exposes a dictionary-entry
escape hatch that keeps pdfTeX's verbatim attribute fragments inside the
crate-owned dictionary framing. `pdf_writer`
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
finalization barrier as DVI and HTML. Unmapped TeX fonts use a host-neutral PK
resource boundary. A typed request contains the TeX name, resolved DPI, and
frozen mode and names the exact `name.<dpi>pk` resource; native filesystem
lookup is only one provider for that request. The bounded decoder in
`tex-fonts` validates short, extended, and long character packets and
normalizes packed or raw rasters to immutable row-byte masks before
checkpointed provision to `Universe`. Finalization selects only an
already-provided exact resource and emits a Type3 font with one CharProc per
used glyph. Both request and decoded program types live below the native
CLI/WASM boundary and perform no filesystem or process access.

Finalization also builds pdfTeX's default document-information dictionary and
registers it in the trailer through `pdf_writer`. `/Producer`, `/Creator`,
`/Trapped`, and the `PTEX.Fullbanner` compatibility entry are deterministic;
`/CreationDate` and `/ModDate` use the immutable pinned job clock. The live
final values of `\pdfomitinfodict`, `\pdfinfoomitdate`,
`\pdfsuppressptexinfo`, and `\pdfptexuseunderscore` select omission and the
legacy `PTEX.` versus `PTEX_` spelling exactly as in pdfTeX (PDF 2 also selects
the underscore spelling). `\pdfomitprocset` is captured in each committed
page receipt because resource dictionaries are page-scoped: negative values
always emit `/ProcSet`, zero emits it before PDF 2, and positive values omit
it. Both the information and resource dictionaries remain structural values
whose final bytes are written only by `pdf_writer`.

pdfTeX mode freezes `\pdfoutput`, the PDF version, stream/object compression,
decimal precision, gamma conversion, draft mode, inclusion copy-fonts, PK
resolution, and unique-resource-name policy at the first committed shipout.
Invalid major/minor
versions recover to 1.4 with the pinned diagnostics; compression and precision
use pdfTeX's bounded first-write values. Later output-mode, version, or draft
mode changes are fatal setup errors. `\pdfdecimaldigits` controls page-box and rule-number
rounding, and DVI plans remain byte-identical whether PDF mode is selected or
not.

External images cross the execution boundary as immutable typed sources, not
filesystem handles. `\pdfximage` records raster metadata or a selected PDF
page and `\pdfrefximage` becomes a positioned artifact effect. Raster PNG and
JPEG sources lower to typed image XObjects; alpha is a separately reserved
soft mask. Included PDF pages lower to form XObjects with selected-page-box
translation, recursively remapped inherited resources, and typed transparency
group references. Repeated references reuse the registered object. The
serializer uses `pdf_writer`'s typed image, form, resources, page, and content
builders; imported dictionaries are converted to the detached typed value
model before serialization, so `lopdf` is only an input parser.

Image configuration is consumed at its pdfTeX scope. The live image resolution
sets missing raster DPI, gamma and high-color controls transform PNG samples,
and explicit/live/obsolete page-box controls are resolved while scanning the
image request. Included-PDF versions follow the signed inclusion error level.
The frozen unique-resource setting selects deterministic content-derived
XObject names. Draft mode suppresses publication without truncating an
existing destination. Upstream `\pdfinclusioncopyfonts=0` substitutes only an
embedded Type-1 font that matches a host font-map entry; the detached importer
has no such candidate and therefore copies the included graph for either value.

## Parity oracle

`tests/corpus/tex_exec/pdf_output_policy` is regenerated with pinned pdfTeX
1.40.27 in INITEX mode. It commits the output-control defaults, TeX grouping,
first-write range recovery, warning text, and recovered version. Hermetic
Umber tests pair that fixture with alias-cell, fatal setup, version header,
object-stream/type-2-xref, decimal rounding, and DVI/PDF switching assertions.

Committed `tests/corpus/pdf` fixtures are regenerated only through
`scripts/regen-fixtures.sh --area pdf` or its `--case pdf/<case>` form. The
reference producer is pinned pdfTeX 1.40.27 with the repository's deterministic
clock policy. Structural comparison
parses both files, removes byte-layout-only fields (offsets, xref spelling,
compression containers, and permitted volatile metadata), resolves indirect
references, and compares the normalized catalog/page/resource/content graph.
It must not discard drawing operators, geometry, font selection, resource
names, page order, or stream bytes after decompression.

Rendering uses Poppler `pdftoppm` 25.08.0 at 72 dpi in grayscale mode. The
regenerator requires exact PGM equality for ordinary fixtures. Embedded-font
and PK fixtures use a maximum gray-value delta of two for independent font
rasterizers and additionally require exact UTF-8 extraction. The committed
300 and 600 DPI PK-only cases pin real bitmap programs, Type3 structure, and
resolution-dependent rendered output. Each attestation records the renderer
and extractor arguments plus SHA-256 identities of both PDFs, the reference
raster, and extracted text. Ordinary cargo tests remain hermetic: they
reproduce the committed Umber PDF
bytes, repeat structural normalization against the committed pdfTeX PDF, and
verify the raster attestation chain without launching pdfTeX or Poppler. A
structural mismatch cannot be blessed by a visually similar page. Tests also
run the complete committed DVI corpus byte-for-byte. The
`object_dictionaries` case composes reserved, referenced, immediate, and stream
objects with catalog, open-action, names, info, trailer, and trailer-ID input.
Its retained-session regression rolls back and replays the source, requiring
identical PDF bytes and finalized engine-state hashes.

The normalized graph deliberately hides indirect-object numbers because
writer layout is not semantic structure. A separate focused oracle asserts the
observable identities: the composed case allocates raw objects 1 and 2, its
action and forward page as 3 and 4, page resources and content as 5 and 6, and
the page tree, Names, catalog, and Info dictionaries as 7 through 10. It also
uses `useobjnum 1` and `\pdfrefobj 1`, so explicit identity preservation is not
masked by normalization.

## Delivery gates

1. Detached model, validation, canonical semantic identity, and this design.
   **Done.**
2. Valid deterministic PDF serialization for a minimal page. **Done.**
3. pdfTeX shipout integration and checkpointed engine ledger. **Done.**
4. Normalized pdfTeX structure fixtures, rendered-page fixtures, and the full
   DVI regression gate. **Done.**

The parent backend issue is complete only when all four gates pass.
