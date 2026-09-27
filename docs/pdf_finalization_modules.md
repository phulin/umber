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
The Umber input adapter decodes one committed page or form at a time when
collecting font resources and physical character uses. Only the font summary
survives that iteration; decoded page trees and positioned events are released
before the next artifact. Pages precede forms, forms retain object-number
order, duplicate font identities retain the last resource, and character uses
retain their first-use watermark. Encoded artifacts remain owned by the final
input for later content lowering. This bounds temporary decoded material by
the largest artifact instead of the whole document.

The version-24 artifact codec and its byte format are separate. Validation uses
format-specific font and image observations plus the existing PDF parity and
validator gates; deterministic serialization remains an exact-byte contract.

## Immutable artifact byte ownership

Serialized page and form artifacts use `SharedBytes` after their content hash
is established. The verified commit payload, in-memory artifact store, committed
output ledger, detached completion, and PDF finalization input share immutable
byte owners across their handoffs. Independently constructed artifacts may
have separate owners even when their content hashes match. Cloning a receipt
copies its metadata and byte handle;
it does not duplicate the serialized page. Real-file reads still authenticate
the loaded bytes before publishing their owner.

This sharing is confined to frozen output bytes. It introduces no aliases to
runtime node records, mutable annex storage, or TeX registers. The existing
node-region transfer and explicit-copy rules remain unchanged. A rollback may
discard a publication or restore a prior ledger while another accepted receipt
keeps its bytes alive; no consumer can mutate those bytes. Render provenance
and effect occurrence metadata retain their existing independent ownership and
rollback rules. The artifact codec, content hashes, publication order, and PDF
object allocation are unchanged.
