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
