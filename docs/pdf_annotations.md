# pdfTeX annotations and running links

This document fixes the implementation contract for `\pdfannot`,
`\pdfstartlink`, `\pdfendlink`, `\pdfrunninglinkon`,
`\pdfrunninglinkoff`, `\pdflastannot`, `\pdflastlink`, and
`\pdflinkmargin`. The reference is pdfTeX 1.40.27 from TeX Live 2025. The
primary sources are the annotation and linking sections plus the formal
grammar in the pdfTeX manual, the 1.40.22 NEWS entry, and the public-domain
`tests/15-startlink-boxing` and `tests/16-nolink-special` regression cases.

## Language and state contract

`\pdfannot` is valid in horizontal, vertical, and math modes. Its grammar is
either `reserveobjnum`, which only reserves an object and updates
`\pdflastannot`, or an optional `useobjnum <positive integer>`, zero or more
`width`, `height`, and `depth` clauses in any order, followed by expanded
general text. Omitted dimensions are _running_, not zero. A normal annotation
allocates its object while scanning and appends a whatsit; it is emitted only
if that whatsit reaches shipout. `useobjnum` must consume a compatible reserved
object and must not allocate a replacement. A focused 1.40.27 probe observes
initial `\pdflastannot=0`, reservation changing it to 1, and later use of that
reservation leaving it at 1.

`\pdfstartlink` is valid in horizontal and math modes. It scans zero or more
dimension clauses, an optional `attr {<expanded general text>}`, then the
shared `user`, `goto`, or `thread` action grammar. It allocates a fresh link
annotation object immediately, updates `\pdflastlink`, and appends a start
whatsit. `\pdfendlink` appends the paired end whatsit. The last enquiries are
read-only integers, initially zero, and participate in snapshots through the
PDF allocation ledger rather than grouped environment assignments. A focused
probe observes an annotation reservation as object 1 and the following link
as object 2; the shipped page owns `/Annots [1 0 R 2 0 R]` in encounter order.

The start and end must be scanned at the same box-nesting level. An end with no
open link is an execution error. Since pdfTeX 1.40.22, an end encountered at a
different level emits the exact warning prefix
`pdfTeX warning: \\pdfendlink ended up in different nesting level than \\pdfstartlink`
and continues. The recovery closes the active link at the encountered end so
the engine does not retain a poisoned open-link state. Nested starts are kept
as a stack and paired last-in-first-out.

`\pdflinkmargin` is the existing grouped dimension parameter, with a 0pt
default. Its value is deliberately read when a containing page is shipped,
not when a link is scanned. `\pdfrunninglinkoff` and `\pdfrunninglinkon` append
ordered whatsits; they are not assignments and therefore do not group. The
shipout flag starts enabled for every page. A control affects subsequently
entered horizontal boxes, which is why a control at the beginning of a parent
box can suppress a nested box while a control within that same box is too late
to suppress the box itself.

All eight commands reject PDF-only work when `\pdfoutput <= 0` consistently
with the existing PDF extension diagnostics. Scanners use the shared expanded
keyword, dimension, general-text, and PDF-action scanners so macro expansion,
traced-token recovery, and action diagnostics remain common with catalog
actions.

## Durable representations

The live `PdfState` owns indirect object allocation and last-annotation/link
state. Annotation records contain the object number, three optional dimensions,
and expanded dictionary-entry tokens. Link records additionally contain
expanded attributes and the typed `PdfActionSpec`. Token identities are stored
in checkpointed state and included through their semantic identifiers in the
PDF state fingerprint.

Node whatsits carry typed annotation and link markers. Shipout normalization
copies them into versioned detached page effects; no live `Universe` handle is
allowed downstream. The artifact binary codec, semantic hash, validation, and
replay tests cover every new payload. Link start and end markers retain a
stable logical link identity distinct from an emitted PDF annotation segment:
one source link can yield several indirect annotations after line or page
splitting.

The unexpandable primitive meanings begin at operand 255, leaving 251--254 to
the already reserved parallel branches. The meaning test enumerates every
known operand, proves encode/decode agreement, and rejects duplicate enum
values across the complete unexpandable table rather than only this feature's
range.

## Shipout geometry and continuation

Positioned traversal emits explicit horizontal-box entry and exit events with
nesting depth, plus positioned annotation/link/control markers. Box geometry
uses the same glue-adjusted coordinates as text and rules. This event stream is
interpreted in page order by one document-level link lowerer, allowing active
links to survive an artifact and page boundary without consulting mutable TeX
state.

An annotation rectangle starts at the marker position. Each explicit dimension
uses its scanned value; each running dimension is taken from the containing
box. PDF coordinates add the committed page origin, invert the vertical axis,
and apply magnification and deterministic decimal rounding through the existing
PDF geometry helpers.

For a link with explicit width, one segment is made from the start marker using
that width. A running-width link makes a partial segment from its start marker
to the right edge of the containing horizontal box, full segments for later
horizontal boxes at the same nesting level while it remains active, and a
partial final segment from the left edge to the end marker. The active stack is
carried into later pages, so intermediate and final page segments use the same
attributes and action but distinct indirect objects. Running height/depth are
resolved from each contributing box; explicit values remain fixed. The
shipout-time link margin expands all four rectangle edges and may therefore
differ between pages containing one logical link.

Running-link suppression prevents only automatically propagated full-box
segments. It does not discard the explicit start/end markers or change pairing.
The suppression flag resets to enabled before each page, matching the manual.
Empty or negative rectangles follow pdfTeX's reference structure observations
and are pinned before corpus acceptance rather than silently normalized.

The allocation ledger reserves the first segment at `\pdfstartlink`, which is
the value returned by `\pdflastlink`. Additional continuation segments reserve
objects deterministically during page commit in traversal order. Every emitted
segment belongs to exactly one page `/Annots` array. No annotation object is a
page resource and no annotation may be shared between copied boxes; copying a
box creates fresh shipped segment objects even though the typed source payload
is reused.

## Typed PDF boundary

The detached PDF graph validates that every page `/Annots` entry references an
existing annotation dictionary and that an annotation is owned by one page.
The page serializer calls `pdf_writer`'s typed page-annotation and annotation
builders. Rectangle, subtype, action, and known attributes use typed methods.
The vendored crate is extended with narrowly typed APIs when its public surface
cannot represent a required pdfTeX dictionary operation.

User-provided annotation body and `attr` fragments remain explicit compatibility
escape hatches at the engine boundary, like existing raw pdfTeX catalog/object
fragments. Generated `/Type /Annot`, `/Rect`, `/Annots`, action dictionaries,
references, arrays, and operators are never assembled as raw PDF syntax in
`tex-out` or the Umber driver.

## Parity evidence

The committed `annotations_running` corpus separates scanner/diagnostic
observations from PDF output parity. Its two pages pin `/Annots` encounter
order, unique page ownership, general annotation geometry, running-link page
splitting, and shipout-time margin changes against pdfTeX 1.40.27. Exact Umber
bytes pin the typed serializer and deterministic allocation; the committed
Poppler grayscale render and renderer attestation pin page geometry. Ordinary
tests consume only these committed artifacts. Focused unit tests additionally
pin copied-occurrence allocation, explicit box-boundary traversal, and
running-link controls without invoking external tools.
