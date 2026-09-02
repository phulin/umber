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

Pool-stable logical tables belong in sibling `logical_node_table.rs` and
aggregate ownership marks or transfer receipts belong in sibling
`node_envelope.rs`. Do not add either concern to this codec tree. Until the
atomic production cutover, this module remains non-resident proof code and
must not create a second live node representation or alter production backing.

Preserve crate-private codec visibility and explicit integer encoding. Do not
transmute records, serialize native bytes, add owned payloads to `NodeRecord`,
or weaken stale annex-coordinate validation. Run the focused `node_record`
tests and the owning `tex-state` suite after changes.
