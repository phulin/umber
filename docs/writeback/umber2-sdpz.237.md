# umber2-sdpz.237: canonical ToUnicode CMaps

The representative remains complete unmodified recent-arXiv row
`2605.22212`, entrypoint and source-derived jobname `paper-JDE-o4`. Its
34,138-byte entrypoint has SHA-256
`d10e871e029a353935fe48f0ddd9119ea1dfb1b90a27875cd6fbc7298931c51c`.
The schema-8 distribution manifest has SHA-256
`6ae7fb0a5ae6b7901c55e804cce07d2c353713f3ca8207d84c6ea1e64a3bbb5a`
and root aHash64 `cf7099ef97710816`. Umber loaded the unchanged 1,126,714-byte
schema-12 pdfLaTeX object at `ahash64-v1-4225c474b34108a4`, SHA-256
`e02298271a57b081d055fa7c754a7d78f220d29618ca9c006e695c51cdb146af`.

pdftex.web section 32e delegates mapped-font ToUnicode construction to
`tounicode.c::write_tounicode`. That writer scans all 256 slots of the
original external or built-in encoding, not only the used subset. It names the
CMap from the TFM and encoding names, resolves a TFM-scoped glyph mapping
before the global namespace, composes underscore-separated glyph components,
and separates strings from consecutive scalar ranges. Its range construction
does not increment the last UTF-16 byte beyond 255, emits at most 100 entries
per group, and retains the prologue, empty groups, and resource footer.

Detached finalization now keeps the original Type-1 program's encoding beside
the subset program written to `FontFile`, resolves all 256 encoding slots, and
serializes the canonical CMap shape before the existing zlib compressor runs.
Focused controls cover the exact resource framing, string/range separation,
the UTF-16 final-byte boundary, suffix and component resolution, an unused
built-in slot, and TFM-scoped precedence over a later global definition. The
committed subset Type-1 and TrueType fixtures pin the new stream shape.

The authenticated offline row used `SOURCE_DATE_EPOCH=1772323200`,
`FORCE_SOURCE_DATE=1`, 500,000,000 expansion fuel, 10,000,000 execution steps,
a 120-second wall guard, two-second termination grace, and the authorized
2,048-MiB aggregate-RSS ceiling. The optimized Umber binary has SHA-256
`6ec6f8bfe139197a20c6d66f0ba6b407bb3e2997897189741e9aaafe1a08abec`;
its 298,243-byte PDF has SHA-256
`f38b800216bcb24e85dced194763e50934991827e44c1d74e6eab740570c1b1f`.
F31's raw ToUnicode stream is now exactly 740 bytes with SHA-256
`747613b0b34a24c7b1cdc3f60b3ca882bf212c097ca9c25b11a7176536503fb5`;
its decoded 1,719 bytes have SHA-256
`2f65dcadf13bec4ced08634804f3da546085ad7e1325dce7c94e2793903288f7`.

F31 still omits `/Name`, retains `/StemV 79`, and retains its exact 7,948-byte
raw `FontFile` stream with SHA-256
`f5cfbe11a9d7ad7ca22266ff08e1babd7014329a312574ce5523ab07deea0bae`.
The PDF-mode AUX and OUT remain byte-exact at
`046f06e5c9423f147955408da40f62287acdb46ff329e8e1bd74ecd987b37ae9`
and
`a7c9a980b0351fbf9cf761e238b022a40e013b2b21bd1258f369f870303210aa`.
The loaded format and pure finalizer boundary remain unchanged, as does the
preceding exact nine-page normalized DVI: 64,380 bytes, raw SHA-256
`d35269fbf6c2b4b5303b22194cde5de063d9af25e480f0c8a83636670eb0e96c`.

The normalized projection advances from
`06e4380735d747d5ab6025a366e16118c487184b0f04afac193b72878d2e271d`
to
`3f205c4d2f4a9b242047ca655a17c2e2b51174563b39f8a3ccc21b95ad537ad1`;
the clean projection remains
`038c47236c87790cb5c18b008911fbc98a14eed943e1cd3209a9e2e920dcdbf5`.
The first remaining independent normalized difference is F32's embedded
Type-1 program: clean pdfTeX emits 25,245 raw bytes with `/Length2 23712` and
SHA-256
`18a3eef3cdd18710e0f0d99ede535f6cab37a6639c1a8c84836284d13ac6331`;
Umber emits 25,246 raw bytes with `/Length2 23713` and SHA-256
`49661329146e16f7e137169af11bff81771c1a3bc42727d6729d9886a0c53486`.
