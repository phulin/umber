# pdf-writer provenance

This directory is a source fork of `pdf-writer` 0.15.0, published by the
Typst project at <https://github.com/typst/pdf-writer> and downloaded from the
crates.io package with SHA-256 registry checksum
`f5e456864a7a304047bff84977dc6fb162bd956475d40ba50b2dcecaada7f753`.
The unmodified upstream release is dual-licensed under
MIT or Apache-2.0; both license texts are retained beside this file.

Umber carries a minimal extension that lets `Chunk` register compressed
objects, builds true PDF object streams through `Obj`, and emits type-2 entries
from `Pdf::finish_with_xref_stream`. The extension is intentionally contained
in the upstream crate so all PDF object, xref, stream-dictionary, trailer, and
framing syntax remains authored by `pdf_writer`.

Umber additionally carries a minimal typed `Content::inline_image` builder for
the `BI` dictionary, `ID` binary payload, and `EI` terminator. Upstream 0.15.0
contains an explicit inline-image TODO; keeping the extension here prevents
Type-3 PK glyph procedures from hand-authoring PDF framing downstream.

Umber also adds `Content::verbatim_operations` as a deliberately narrow
escape hatch for user-authored pdfTeX `\pdfliteral` payloads. The method owns
content-stream separation/framing; all engine-generated operators continue to
use the typed writer API.

Umber additionally carries typed pdfTeX navigation writers: nullable XYZ
destinations, string-key name-tree limits and named-destination dictionaries,
indirect actions and outline references, and article thread/bead structures.
`PdfStringSyntax` confines pdfTeX's legacy literal/hex/bare string policy to a
single primitive object writer; downstream navigation code does not author
dictionary, array, reference, or string framing.
