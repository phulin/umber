# umber2-sdpz.236: scalable font-dictionary identity

The representative remains complete unmodified recent-arXiv row
`2605.22212`, entrypoint and source-derived jobname `paper-JDE-o4`. Its
34,138-byte entrypoint has SHA-256
`d10e871e029a353935fe48f0ddd9119ea1dfb1b90a27875cd6fbc7298931c51c`.
The schema-8 distribution manifest has SHA-256
`6ae7fb0a5ae6b7901c55e804cce07d2c353713f3ca8207d84c6ea1e64a3bbb5a`
and root aHash64 `cf7099ef97710816`. Umber loaded its 1,126,714-byte
schema-12 pdfLaTeX object at `ahash64-v1-4225c474b34108a4`, SHA-256
`e02298271a57b081d055fa7c754a7d78f220d29618ca9c006e695c51cdb146af`.
The paired clean pdfTeX format remains SHA-256
`d557a9b8d7de1ff9bc5a570830701417d32f09b300f0188f9a18a485aaf1ff8e`.

pdftex.web section 32e delegates real mapped fonts to
`writefont.c::write_fontdictionary`. That scalable Type-1/TrueType writer
emits `/BaseFont` but no `/Name`; the separate `writet3.c::writet3` path
explicitly emits the resource-local `/Name /F...`. The canonical rule is
therefore subtype-wide, not conditional on CMR7, subsetting, embedding, or
resource number.

Detached finalization now constructs every font-dictionary header through one
typed scalable-versus-Type-3 boundary. Type-1, TrueType, and resident mapped
dictionaries omit `/Name`; PK and generated fallback-space Type-3 dictionaries
retain it. Focused controls exercise Type-1 and TrueType omission plus Type-3
presence. The seven committed scalable-font fixtures pin omission against
pdfTeX, while the 300/600-DPI PK fixtures and tagged-space fixture retain their
Type-3 `/Name` evidence.

The final optimized Umber binary has SHA-256
`84d1cc6c905a917a57d98cd124a288f2c5b7d6b3b3af067fcf145846f9b3c82d`.
The authenticated offline row used `SOURCE_DATE_EPOCH=1772323200`,
`FORCE_SOURCE_DATE=1`, 500,000,000 expansion fuel, 10,000,000 execution steps,
a 120-second wall guard, two-second termination grace, and the separately
authorized 2,048-MiB aggregate-RSS ceiling.

The final 289,555-byte PDF has SHA-256
`53a84d583118a9c9a74df88815eb1f14ad23d3fe78672d3d770559c74b9f4f82`.
F31 now proceeds directly from `/LastChar 102` to `/Subtype /Type1`; `/Name
/F31` is absent and `/StemV 79` remains exact. Its raw Flate stream remains
exactly 7,948 bytes with SHA-256
`f5cfbe11a9d7ad7ca22266ff08e1babd7014329a312574ce5523ab07deea0bae`,
and the decoded Type-1 program remains exactly 8,435 bytes with SHA-256
`bf3d13103e1e13aad33ae5c2eb99a84d11691f45cfa880b05f464dae9f7aff7a`.
The normalized projection advances from
`f094f3e5e11850e4aa07e8170f7b3eee5d457c83716debbbfd8c4a946617918f`
to
`06e4380735d747d5ab6025a366e16118c487184b0f04afac193b72878d2e271d`;
the clean projection remains
`038c47236c87790cb5c18b008911fbc98a14eed943e1cd3209a9e2e920dcdbf5`.

The PDF-mode AUX and OUT remain byte-exact at
`046f06e5c9423f147955408da40f62287acdb46ff329e8e1bd74ecd987b37ae9`
and
`a7c9a980b0351fbf9cf761e238b022a40e013b2b21bd1258f369f870303210aa`.
The format is loaded unchanged, and this detached PDF-only finalizer change
cannot alter the preceding exact nine-page normalized DVI: 64,380 bytes, raw
SHA-256
`d35269fbf6c2b4b5303b22194cde5de063d9af25e480f0c8a83636670eb0e96c`.

The first remaining independent normalized difference is F31's ToUnicode
stream. Clean pdfTeX emits a 740-byte raw Flate stream with SHA-256
`747613b0b34a24c7b1cdc3f60b3ca882bf212c097ca9c25b11a7176536503fb5`;
Umber emits 258 raw bytes with SHA-256
`78bdff743df9e42c06be44b0f2cc1b37ed9a15c2f682a36449eb3d710df0631e`.
Its successor is tracked separately.
