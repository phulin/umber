# umber2-sdpz.235: canonical Type-1 `/StemV`

The representative remains complete unmodified recent-arXiv row
`2605.22212`, entrypoint and source-derived jobname `paper-JDE-o4`. The
11,816-byte archive has SHA-256
`6af819f487f2912c4b12f34fd5c4ba7e512626245d125a2deb00bf97f4f99a59`;
the 34,138-byte entrypoint has SHA-256
`d10e871e029a353935fe48f0ddd9119ea1dfb1b90a27875cd6fbc7298931c51c`.
The schema-8 distribution manifest has SHA-256
`6ae7fb0a5ae6b7901c55e804cce07d2c353713f3ca8207d84c6ea1e64a3bbb5a`
and root aHash64 `cf7099ef97710816`. Umber loaded its 1,126,714-byte
schema-12 pdfLaTeX object at `ahash64-v1-4225c474b34108a4`, SHA-256
`e02298271a57b081d055fa7c754a7d78f220d29618ca9c006e695c51cdb146af`.
The paired clean pdfTeX format has SHA-256
`d557a9b8d7de1ff9bc5a570830701417d32f09b300f0188f9a18a485aaf1ff8e`.

pdftex.web section 799 delegates font construction to `write_fontstuff`.
Pinned `writefont.c::preset_fontmetrics` first initializes `/StemV` from one
third of the active TFM period width on pdfTeX's three-place `dividescaled`
raster. `writet1.c::t1_scan_param` then replaces that fallback when the Type-1
program supplies `/StdVW`. CMR7's TFM fallback is `108`, while its encrypted
private dictionary supplies `/StdVW [79]`; the latter is therefore the
canonical descriptor value. Umber previously inspected only the subset's
cleartext and fell back to a constant `80`.

`PdfType1Program` now scans `/StdVW` first in cleartext, then in the decrypted
eexec private-dictionary prelude, stopping before Subrs or CharStrings binary
data. The finalizer uses that one program-owned value and falls back to the
TFM period-width calculation only when it is absent. A bounded negative
control rejects comment text, a prefixed `/OtherStdVW`, and a key-shaped byte
sequence after the binary boundary. The committed CMR10 control retains
`/StdVW 69` before and after subsetting, while a separate finalizer control
proves an explicit value overrides the period-derived fallback.

The final optimized Umber binary has SHA-256
`f74f2f70a42650942e0bac67027d5b79b60cfd1a7e163e535cf7f6af392e4a8a`.
The authenticated offline PDF run used `SOURCE_DATE_EPOCH=1772323200`,
`FORCE_SOURCE_DATE=1`, 500,000,000 expansion fuel, 10,000,000 execution
steps, a 120-second wall guard, and two-second termination grace. The
currently integrated node-memory regression tracked by `umber2-66p0.8.40.113.5.7`
required the coordinator-authorized temporary 2,048-MiB RSS ceiling; the
unchanged standing 1,536-MiB ceiling stopped this base at 1,590,636--1,602,324
KiB before producing an artifact.

The final 289,621-byte PDF has SHA-256
`e26f720073481a4203dfa5c924668eaa6e6b6c3809895519adbb29d5961fe195`.
F31 now has `/StemV 79`, exactly matching clean pdfTeX. Its raw Flate stream
remains exactly 7,948 bytes with SHA-256
`f5cfbe11a9d7ad7ca22266ff08e1babd7014329a312574ce5523ab07deea0bae`,
and the decoded Type-1 program remains exactly 8,435 bytes with SHA-256
`bf3d13103e1e13aad33ae5c2eb99a84d11691f45cfa880b05f464dae9f7aff7a`.
The normalized projection advances from
`0d14dcf777a485e5736b6b2c11328fbbe433bc3c6ed35000879ebeb5ec4e3cd0`
to
`f094f3e5e11850e4aa07e8170f7b3eee5d457c83716debbbfd8c4a946617918f`;
the clean projection remains
`038c47236c87790cb5c18b008911fbc98a14eed943e1cd3209a9e2e920dcdbf5`.

The PDF-mode AUX and OUT remain byte-exact at
`046f06e5c9423f147955408da40f62287acdb46ff329e8e1bd74ecd987b37ae9`
and
`a7c9a980b0351fbf9cf761e238b022a40e013b2b21bd1258f369f870303210aa`.
The change is confined to detached Type-1 PDF inspection and finalization;
the preceding exact nine-page normalized DVI remains 64,380 bytes with raw
SHA-256
`d35269fbf6c2b4b5303b22194cde5de063d9af25e480f0c8a83636670eb0e96c`,
and the existing format was loaded unchanged by the accepted row.

The focused `tex-fonts` and `tex-out` suites pass 84 and 169 tests. The full
`cargo test -q --tests` routine suite also passes. The first remaining
independent normalized difference is F31's extra `/Name /F31` font-dictionary
entry in Umber; clean pdfTeX proceeds directly from `/LastChar 102` to
`/Subtype /Type1`. Its successor is tracked separately.
