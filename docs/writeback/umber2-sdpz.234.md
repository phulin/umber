# umber2-sdpz.234: canonical PDF stream compression

The representative remains complete unmodified recent-arXiv row
`2605.22212`, entrypoint and source-derived jobname `paper-JDE-o4`. The
34,138-byte entrypoint has SHA-256
`d10e871e029a353935fe48f0ddd9119ea1dfb1b90a27875cd6fbc7298931c51c`.
The schema-8 distribution manifest has SHA-256
`6ae7fb0a5ae6b7901c55e804cce07d2c353713f3ca8207d84c6ea1e64a3bbb5a`
and root aHash64 `cf7099ef97710816`. Umber loaded its 1,126,714-byte
schema-12 pdfLaTeX object at `ahash64-v1-4225c474b34108a4`, SHA-256
`e02298271a57b081d055fa7c754a7d78f220d29618ca9c006e695c51cdb146af`.
The paired clean pdfTeX format has SHA-256
`d557a9b8d7de1ff9bc5a570830701417d32f09b300f0188f9a18a485aaf1ff8e`.

The previous implementation supplied `flate2`'s default `miniz_oxide`
backend with the correct level and produced a valid, deterministic stream,
but DEFLATE containers are not uniquely encoded. For the exact 8,435-byte
F31 Type-1 program, the old 7,948-byte stream had SHA-256
`aad39cb5854a355f56be7fb976774bb95a7ac4007db4565b7f4a24e2423c6599`.
Recompressing those same bytes with stock zlib 1.3 at level 9 immediately
produced the clean pdfTeX stream's 7,948 bytes and SHA-256
`f5cfbe11a9d7ad7ca22266ff08e1babd7014329a312574ce5523ab07deea0bae`.

pdftex.web section 685 owns PDF stream buffering and compression state:
ordinary input reaches `write_zip(false)` and stream completion reaches
`write_zip(true)`. Pinned `writezip.c::writezip` implements those calls with
zlib `deflateInit(level)`, `Z_NO_FLUSH`, and `Z_FINISH`, using a 32 KiB output
buffer. The pinned TeX Live build carries zlib 1.3.2. Umber now selects
`flate2`'s zlib backend and activates `libz-sys` 1.1.29's static stock-zlib
feature, whose bundled source is the same zlib 1.3.2 release. The existing
serializer remains the one owner for ordinary, form, object, and
cross-reference stream compression.

The focused 8 KiB binary control pins exact zlib stream length and SHA-256 at
levels 1, 6, and 9, then independently inflates each stream to the original
bytes. Restoring the former miniz backend fails the level-9 control before the
digest check because its result is 5,887 rather than 5,888 bytes. The level-6
and level-9 zlib streams have the same length but different digests, which
also proves the selected compression level is not ignored. The existing
filter-conflict, pre-encoded-stream, invalid-level, deterministic-repeat, and
decoded-byte controls remain unchanged. A wasm32 compile confirms the static
stock-zlib selection retains the browser target.

The optimized test-profile Umber binary has SHA-256
`db118af1cbc16c7b13561bf033d1804be0b651054f26de1e1f9c835082d3cb7c`.
The authenticated PDF run used `SOURCE_DATE_EPOCH=1772323200`,
`FORCE_SOURCE_DATE=1`, offline mode, 500,000,000 expansion fuel, 10,000,000
execution steps, and the 120-second/1,536-MiB guard with two-second termination
grace. It emitted a 289,560-byte PDF with SHA-256
`a78f72846ad04063ee09003e0a400b7fc2e5e859eccf106407475dff041dbf78`.
F31 still declares `/Length 7948 /Length1 1480 /Length2 6955 /Length3 0`.
Its raw Flate stream is now exact at SHA-256
`f5cfbe11a9d7ad7ca22266ff08e1babd7014329a312574ce5523ab07deea0bae`,
and its decompressed 8,435 bytes remain exact at SHA-256
`bf3d13103e1e13aad33ae5c2eb99a84d11691f45cfa880b05f464dae9f7aff7a`.

The normalized PDF advances from
`c8f89db0f3f273aba346726802428332c3cdd19c61309632dad030acad73b362`
to
`0d14dcf777a485e5736b6b2c11328fbbe433bc3c6ed35000879ebeb5ec4e3cd0`.
The clean projection remains
`038c47236c87790cb5c18b008911fbc98a14eed943e1cd3209a9e2e920dcdbf5`.
Its first remaining difference is now F31 `/StemV 79` in clean pdfTeX versus
`/StemV 80` in Umber; that independent successor is tracked separately.

The fresh DVI-mode run is byte-identical to the prior 64,380-byte Umber DVI at
SHA-256
`d35269fbf6c2b4b5303b22194cde5de063d9af25e480f0c8a83636670eb0e96c`.
Its 5,069-byte AUX and 3,242-byte OUT remain byte-exact at
`fc8187a62d66973d7288246dca70c3dbe35ee805cb0702f1e12ebc6a18e2eb07`
and
`a7c9a980b0351fbf9cf761e238b022a40e013b2b21bd1258f369f870303210aa`.
The PDF-mode AUX and OUT likewise remain exact at
`046f06e5c9423f147955408da40f62287acdb46ff329e8e1bd74ecd987b37ae9`
and the same OUT hash. Loading the existing format through the new backend
therefore preserves format behavior as well as every pre-PDF output channel.
