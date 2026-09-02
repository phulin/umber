# umber2-sdpz.232: Type-1 trailer line normalization

The representative remains complete unmodified recent-arXiv row
`2605.22212`, entrypoint and source-derived jobname `paper-JDE-o4`. The
34,138-byte entrypoint has SHA-256
`d10e871e029a353935fe48f0ddd9119ea1dfb1b90a27875cd6fbc7298931c51c`.
The authoritative current distribution is the schema-8 tree at
`target/umber2-sdpz.233/distribution`: its manifest has SHA-256
`6ae7fb0a5ae6b7901c55e804cce07d2c353713f3ca8207d84c6ea1e64a3bbb5a`
and root aHash64 `cf7099ef97710816`. Umber loaded the 1,126,714-byte
schema-12 pdfLaTeX object `ahash64-v1-4225c474b34108a4`, SHA-256
`e02298271a57b081d055fa7c754a7d78f220d29618ca9c006e695c51cdb146af`.
The paired clean pdfTeX 1.40.29 pdfLaTeX format has SHA-256
`d557a9b8d7de1ff9bc5a570830701417d32f09b300f0188f9a18a485aaf1ff8e`.

`pdftex.web` section 799 delegates embedded-font output to
`write_fontstuff`. In pinned pdfTeX 1.40.29,
`writet1.c::t1_subset_end` reads the post-CharStrings suffix through the
structural `mark currentfile closefile` line with `t1_getline`, the same
bounded text reader used by the Type-1 prelude. Its shared
`ptexmac.h::append_char_to_buf` turns a tab into an ASCII space, rejects a
space at the start of a line, and collapses repeated spaces;
`append_eol` removes a trailing space and canonicalizes the line ending.
The suffix therefore does not preserve the source's leading space before
the close marker. The reader stops at that marker, so the rule does not
extend into binary CharStrings or a following input boundary.

Umber now applies the existing Type-1 ASCII line normalizer only to the
post-CharStrings suffix through that marker. A missing marker retains the
original suffix rather than inventing a boundary. The focused negative
control begins with leading, repeated, tab, trailing, and CR whitespace,
and appends space, tab, and NUL-containing binary data after the marker. It
rejects the old leading-space output while requiring every post-marker byte
to remain exact. The committed CMR10 control additionally asserts the
complete subset program's MD5
`fce9d1c28cd155a89a22e437b3c33f91`, lengths `1391/7337/0`, and exact
newline-delimited close marker.

The three affected committed Umber fixtures advance by exactly one PDF
byte without changing their clean references, raster hashes, or extracted
text. `embedded_subset_type1` is now 10,452 bytes with SHA-256
`8795e068c0a4f750ae6423e349028b26de3f95acdc68306050d28c21707fb1c6`;
its normalized projection has SHA-256
`ebaeb195ed25511a35d21a1e927696e5dda2955f19f2f89846e4713e3b973a40`.
`embedded_subset_omit` and `embedded_subset_controls_negative` are each
8,650 bytes with SHA-256
`d3233bafdb1a8a1682d347164b703a8d0479829910bc297c93f8234184374a9a`;
their normalized projections have SHA-256
`e0c3620a888b9dafd8987c3fda42f97c7ef69d82658b5456dc96aaa01e170f59`.

The complete row ran offline from a fresh directory with
`SOURCE_DATE_EPOCH=1772323200`, `FORCE_SOURCE_DATE=1`, 500,000,000
expansion fuel, 10,000,000 execution steps, and the 120-second/1,536-MiB
guard with two-second termination grace. The fixed executable has SHA-256
`a594fd8b8ab531a1b55067f626a9c2d69bfae99096a0a7c54d2d7d4e653e938e`.
An independent DVI-mode first pass emits the prior exact 64,380-byte Umber
DVI, SHA-256
`d35269fbf6c2b4b5303b22194cde5de063d9af25e480f0c8a83636670eb0e96c`.
The parity harness accepts it against clean pdfTeX after the sole preamble
comment normalization, at normalized SHA-256
`732afeca3dc3ee599a8b7357b5896f5ce1f7c6143608ecc1dd41ee2e85a29f6a`.
The 5,069-byte DVI-mode AUX and 3,242-byte OUT remain byte-exact at
`fc8187a62d66973d7288246dca70c3dbe35ee805cb0702f1e12ebc6a18e2eb07`
and
`a7c9a980b0351fbf9cf761e238b022a40e013b2b21bd1258f369f870303210aa`.
The PDF-mode AUX and OUT are also exact with the clean PDF run at
`046f06e5c9423f147955408da40f62287acdb46ff329e8e1bd74ecd987b37ae9`
and the same OUT hash.

The clean PDF remains 284,610 bytes with SHA-256
`dd70d6f6633cdab874b117f1e428f45a054a69d6ed22e2faa68341c34877fc24`;
its normalized projection has SHA-256
`038c47236c87790cb5c18b008911fbc98a14eed943e1cd3209a9e2e920dcdbf5`.
The fixed Umber PDF is 289,840 bytes with SHA-256
`b064416cba5adb3ea1c4f7077ff287cc01ea8d6b8f5453585b74e9ee08a55f8b`;
its normalized projection advances from `.231`'s
`293795a8693898c18abc1398dfffa5b56868bb5d93ef6956dfa873354c420883`
to
`c8f89db0f3f273aba346726802428332c3cdd19c61309632dad030acad73b362`.
F31 now declares `/Length1 1480 /Length2 6955 /Length3 0`. Its complete
8,435-byte decompressed Type-1 program has SHA-256
`bf3d13103e1e13aad33ae5c2eb99a84d11691f45cfa880b05f464dae9f7aff7a`,
byte-exact with clean pdfTeX; this includes the entire previously exact
8,408-byte prefix, selected glyphs, Subr closure, and trailer.

The first remaining normalized F31 difference is outside that program.
Both Flate streams are 7,948 bytes, but clean pdfTeX's raw compressed bytes
have SHA-256
`f5cfbe11a9d7ad7ca22266ff08e1babd7014329a312574ce5523ab07deea0bae`
while Umber's have SHA-256
`aad39cb5854a355f56be7fb976774bb95a7ac4007db4565b7f4a24e2423c6599`.
No compression diagnosis was attempted here. This single next independent
PDF divergence is `umber2-sdpz.234`; later F31 descriptor and ToUnicode
differences remain behind it and were not filed separately.

Validation passed: the focused Type-1 crate suite ran 83 tests, the committed
embedded-font fixture gate passed, `cargo test -q --tests` passed across the
routine workspace suite, and `scripts/check.sh` reported all four gates
passed.
