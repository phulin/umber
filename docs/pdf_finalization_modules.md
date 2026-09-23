# PDF finalization module boundaries

PDF finalization consumes a detached `PdfFinalizationInput` and publishes one
validated, serialized document. `finalize_pdf` retains the ordered allocation
cursor, indirect-object collection, final graph validation, serialization, and
diagnostic publication. Helpers may borrow and advance the cursor or append
objects, but they do not own a second document or publish output separately.

Private modules under `tex-out/src/pdf/finalize/` divide the lowering work by
PDF meaning:

- `navigation` lowers destinations, links, annotations, outlines, and threads.
  It preserves the input object numbers and the existing allocation order for
  generated objects.
- `content` lowers positioned page and form events into content operations and
  resource dictionaries. Pages retain their origin, media box, annotations,
  thread beads, and accessibility policy; forms retain their local geometry,
  nested-form closure, and resource policy. Shared mechanics must not erase
  those differences.
- `fonts` assembles font objects, encodings, subsets, ToUnicode maps, and
  mapped-text metrics. Program identity and font resource identity remain
  separate: a reused program does not merge distinct font metrics or encoding
  decisions.
- `images` decodes raster data and imports PDF page images. The finalizer keeps
  object allocation and deduplication decisions; image helpers return typed
  stream data and dependencies.
- `numeric` holds checked scaled-point and PDF-number conversions used by all
  lowerers. It retains the established rounding and overflow behavior.

The public detached input and PDF graph stay in `finalization.rs` and `pdf.rs`.
The version-24 artifact codec and its byte format are separate. Validation uses
format-specific font and image observations plus the existing PDF parity and
validator gates; deterministic serialization remains an exact-byte contract.
