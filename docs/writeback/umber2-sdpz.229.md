# umber2-sdpz.229: Type-1 subset cleartext streams

The representative remains the complete unmodified recent-arXiv row
`2605.22212`, entrypoint and jobname `paper-JDE-o4`. Its source is 34,138 bytes
with SHA-256
`d10e871e029a353935fe48f0ddd9119ea1dfb1b90a27875cd6fbc7298931c51c`.
This reproduction reused the clean pdfTeX 1.40.29 reference, schema-12 Umber
format, and authenticated TeX Live 2026 distribution recorded by
`umber2-sdpz.228`: format SHA-256
`e02298271a57b081d055fa7c754a7d78f220d29618ca9c006e695c51cdb146af`,
distribution-manifest SHA-256
`2595bf2f5d98f2613ef4456a2f2e3eb1f75c9c4eb324338cafa62a696f5ed20a`,
and distribution root aHash64 `49dd828f7839a1c3`. The run was offline with
`SOURCE_DATE_EPOCH=1772323200`, `FORCE_SOURCE_DATE=1`, 500,000,000 expansion
fuel, and 10,000,000 execution steps. It exited zero with nine pages. Its
generated `.aux` and `.out` have SHA-256
`046f06e5c9423f147955408da40f62287acdb46ff329e8e1bd74ecd987b37ae9`
and `a7c9a980b0351fbf9cf761e238b022a40e013b2b21bd1258f369f870303210aa`.
The PDF-only font-program change cannot affect DVI construction; the prior
nine-page normalized-DVI equality and descriptor metrics therefore remain
unchanged.

Before the fix, page 1 font F31's embedded CMR7 stream was the first normalized
PDF difference. pdfTeX emitted `/Length 7948 /Length1 1480 /Length2 6955
/Length3 0` and raw-stream SHA-256
`f5cfbe11a9d7ad7ca22266ff08e1babd7014329a312574ce5523ab07deea0bae`.
Umber emitted `/Length 9761 /Length1 4286 /Length2 7659 /Length3 545` and
raw-stream SHA-256
`6d73e708dc751f85258d9e54f97c91d3091ec21ed7c6aa7654f17235a5e3b351`.
The decompressed programs first differed at zero-based byte 211, well before
eexec: pdfTeX had collapsed two cleartext spaces while Umber retained both.
The old stream then retained blank and indented lines, the cleartext
`/UniqueID`, every original built-in encoding entry, and the 512-zero PFB
trailer. This rules out PFB binary-segment decoding, eexec conversion, and
CharStrings selection as the first cause. It is the earlier Type-1 cleartext
subsetting rule.

pdftex.web §799 delegates font emission to `write_fontstuff`. In the pinned
pdfTeX 1.40.29 source, `writet1.c::t1_subset_ascii_part` reads cleartext through
the whitespace-normalizing `ptexmac.h` line buffer, removes subset-invalid
`/UniqueID` definitions, and rebuilds a built-in encoding by walking the sorted
requested glyph set and selecting each name's lowest original code.
StandardEncoding is preserved as a predefined encoding. The same writer omits
the 512-zero trailer when `fixedcontent` is false because PDF does not need it.
Umber now applies those generic rules before its existing eexec/CharStrings
pass and records `/Length3 0`.

The focused committed CMR10 negative control requests only `A`, `B`, and `C`.
Its new 1,391-byte clear segment has MD5
`1d2f7f176577933a65bb76a39a86b955` and SHA-256
`f5ef3fcd5fddbde1bc32a98c324ebd5c7b24849f8243ff30ab03874ef4df92f3`,
exactly matching pdfTeX. The test also proves that the old input contains
`/UniqueID 5000793 def` and the unrelated `dup 0 /Gamma` encoding entry, while
the subset rejects both, retains `dup 65 /A put`, and ends its clear segment at
`currentfile eexec`.

The fixed full-row PDF is 321,355 bytes with SHA-256
`f631947bec3d2825a73633e566b2b5fd3f703da4fe20b49733bf5eec6e390438`.
Its normalized projection has SHA-256
`14084483f23199cc5fc65a41fb0e40a469ed4f456aa3e48f06b29305086c114a`.
F31 now emits `/Length 8662 /Length1 1480 /Length2 7659 /Length3 0`, with
raw-stream SHA-256
`2d5b63590258f3674aed6c111da484336c16e256c538334906b154745e77212e`.
The decompressed stream is 9,139 bytes with SHA-256
`81e80da9e43adcb92618018f0d59db8a4646092d4dd9a60b92077628ff232690`.
Its entire 1,480-byte clear segment is byte-exact with the reference, including
SHA-256
`8b108ad4b6a4d809f59f58a3c5f8d2c8286699870c8f4ce9f65241503168ac3f`.
The first remaining program difference is now one-based byte 1,682 in the
encrypted segment, where pdfTeX's independently reduced Subrs/CharStrings
program is 6,955 bytes and Umber's is 7,659 bytes. That next canonical rule is
tracked only in linked successor `umber2-sdpz.230`.
