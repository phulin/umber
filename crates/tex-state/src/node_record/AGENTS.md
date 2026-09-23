# Compact node-codec guidance

Read the repository and `crates/tex-state/AGENTS.md` guidance before editing
this directory.

This directory is the private, storage-independent 32-byte node-record and
typed word-annex codec boundary. Keep the resident record layout, header and
scalar helpers in `layout.rs`; annex keys, markers, fixed payload codecs, and
the standalone annex proof arena in `annex.rs`; outer node codecs in
`node_codec.rs`; whatsit, PDF, byte, and UTF-8 codecs in
`whatsit_codec.rs`; semantic hashing directly over borrowed compact records in
`semantic.rs`; and private round-trip/layout tests in `tests.rs`.

Pool-stable logical coordinates belong in `fork_arena.rs`; aggregate paired
node-and-annex ownership marks and transfer receipts belong in `node_region.rs`.
Do not add either concern to this codec tree. This codec is the production
page-material representation; keep decoding and validation rules shared by
resident traversal and cold materialization.

Preserve crate-private codec visibility and explicit integer encoding. Do not
transmute records, serialize native bytes, add owned payloads to `NodeRecord`,
or weaken stale annex-coordinate validation. Run the focused `node_record`
tests and the owning `tex-state` suite after changes.
