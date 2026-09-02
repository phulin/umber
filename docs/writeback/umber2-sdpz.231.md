# umber2-sdpz.231: Type-1 Subrs closure

The representative remains complete unmodified recent-arXiv row
`2605.22212`, entrypoint and jobname `paper-JDE-o4`. The archive is 11,816
bytes with SHA-256
`6af819f487f2912c4b12f34fd5c4ba7e512626245d125a2deb00bf97f4f99a59`;
the 34,138-byte entrypoint has SHA-256
`d10e871e029a353935fe48f0ddd9119ea1dfb1b90a27875cd6fbc7298931c51c`.

The final reproduction used clean pdfTeX 1.40.29 binary SHA-256
`608cb1760e9a471668ba97eea22fde60f2f7fadd285acd7c7b1ba243ddf71db3`
and its newly rebuilt 2,232,923-byte pdfLaTeX format SHA-256
`d557a9b8d7de1ff9bc5a570830701417d32f09b300f0188f9a18a485aaf1ff8e`;
the format receipt has SHA-256
`bcaa10b08e9300ef1bc32fc22b834a29aca41e835e9d1d7e2090125bab04f4f8`.
Umber used the current schema-12 pdfLaTeX object SHA-256
`ddd082db722654fd9c47e1080d8e128d1a5b0cfb3186f9e3bc01f8943f559ad4`
from the read-only TeX Live snapshot whose manifest has SHA-256
`a68acebc1a83fd4ec0ce8c3baed4e8fe01de9b37e5a878b2f1fc203c2f20662f`,
root aHash64 `df66c327ae636145`, and source-manifest aHash64
`7d0fdcf5b35d0058`. The fixed Umber executable has SHA-256
`a69d924de64a33ef21f609ce787d108f03429cfafb0d5917812263a6bc4d611a`.
All runs were offline with issue-local cache and output, 500,000,000 expansion
fuel, 10,000,000 execution steps, `SOURCE_DATE_EPOCH=1772323200`,
`FORCE_SOURCE_DATE=1`, and the 120-second/1536-MiB/2-second guard.

Before this fix, F31's Type-1 program was exact with pdfTeX through one-based
byte 7,026. The first decrypted semantic difference at byte 7,027 was
`/Subrs 4 array` in pdfTeX versus `/Subrs 18 array` in Umber. Umber retained
all Subrs while its selected `.notdef`, `parenleft`, `parenright`, `one`,
`two`, `three`, `D`, `e`, and `f` CharStrings call none above the conventional
first four.

`pdftex.web` section 799 delegates font emission through `write_fontstuff`.
Pinned pdfTeX 1.40.29 `writet1.c::t1_read_subrs` records the source array by
numeric index and marks entries 0 through 3 without parsing them.
`t1_mark_glyphs` marks `.notdef` before the requested glyph-name tree;
`cs_mark` interprets their Type-1 number and command streams, recursively marks
each `callsubr`, and adds StandardEncoding base/accent CharStrings reached by
`seac`. `t1_flush_cs` does not densely renumber the closure or rewrite
`callsubr` operands. It emits original index order through the highest used
slot, replaces unused holes below that slot with a `lenIV`-sized zero seed and
encrypted `return`, and omits the remaining suffix. The source Subr delimiter
pair (`RD`/`NP`, `-|`/`|`, or either `noaccess put` form) is retained.

Umber now implements that rule inside the existing
`PdfType1Program::subset` owner. Its bounded CharString reader shares the
operand stack across recursive Subr calls, implements pdfTeX's static closure
effects, rejects malformed indices and stack operations, and preserves
original used Subr and glyph program bytes. There is no second font-program
representation.

The synthetic negative control begins with nine Subrs. Selected glyph `A`
calls Subr 7, which calls Subr 5, while unselected glyph `B` calls Subr 8. The
result is `/Subrs 8 array`: slots 0 through 3, 5, and 7 preserve their exact
source programs and operands; holes 4 and 6 are encrypted `return` programs;
and slot 8 is absent. This proves transitive discovery, sparse source ordering
without remapping, conventional first-four retention, and rejection of the old
full-array behavior. The committed CMR10 `A`/`B`/`C` control emits 49 rather
than all 130 Subrs and is byte-exact with pinned pdfTeX from the eexec seed
through `/CharStrings`; that prefix has MD5
`3fded621e7054c5969ec9f611a072a31`.

The current full row's clean PDF remains 284,610 bytes with SHA-256
`dd70d6f6633cdab874b117f1e428f45a054a69d6ed22e2faa68341c34877fc24`;
its normalized projection has SHA-256
`038c47236c87790cb5c18b008911fbc98a14eed943e1cd3209a9e2e920dcdbf5`.
The fixed Umber PDF is 289,861 bytes with SHA-256
`2aa1ddc8992c9a64348bc3e8cada6648afb54a72da95a240d30bb9ccf9318246`;
its normalized projection has SHA-256
`ef9928c72e1623ab0521f7d9b96133e2c976483f072a1d14ea57fbbd2831eb06`.
F31 now declares `/Length1 1480 /Length2 6956 /Length3 0`; its compressed
stream is 7,950 bytes with SHA-256
`2a9c16195465c55808e5607904b1a310f46c885931204b6b9c3bbf5c62eab55e`.
The decompressed program is 8,436 bytes with SHA-256
`874afb844fc1479f446c2fdfe7958f28851a1ea247ef11fad17510d5def2a967`.
The 8,435-byte pdfTeX program has SHA-256
`bf3d13103e1e13aad33ae5c2eb99a84d11691f45cfa880b05f464dae9f7aff7a`.
They are byte-exact through one-based byte 8,408. `t1disasm` shows the sole
remaining font-program difference: Umber retains one leading space before
`mark currentfile closefile`. That later trailer-normalization rule is the
single PDF successor `umber2-sdpz.232`.

The focused embedded CMR10 fixture advances from a 12,037-byte to an
8,729-byte font stream while preserving the raster, extracted text, and clean
reference. Its Umber PDF is 10,453 bytes with SHA-256
`f745c524055ff81765f75e1b7da8f3d770fa3df7a7cea6646fc74b7a3fe7597c`;
the normalized projection has SHA-256
`36bfd97354951548d74c688798e3ba7ec521d0b7b3f0b6ab419dbba36b80f720`.
The same owner also updates the subset-omission and signed-control fixtures;
each now emits an 8,651-byte Umber PDF with SHA-256
`e184adc1a619d3de23940a0936b23949c88abdc82ae0f3cfdd69ee2a5e95304a`
and normalized SHA-256
`62c29703606dfbeda8a3ded8611fad709b7084d9b53614ea7d123bc0067b8bb6`.
The same owner also updates the omit and controls-negative subset fixtures;
both now emit 8,651-byte PDFs with SHA-256
`e184adc1a619d3de23940a0936b23949c88abdc82ae0f3cfdd69ee2a5e95304a`
and normalized SHA-256
`62c29703606dfbeda8a3ded8611fad709b7084d9b53614ea7d123bc0067b8bb6`.

The current embedded format independently exposes a pre-existing page-8 DVI
difference. Clean pdfTeX emits 64,380 bytes with SHA-256
`9e709b574eb25988b594e7b980b1b728f86be3220df3d6a6676e1acb9f20d8ee`;
Umber emits 64,384 bytes with SHA-256
`908187a0b8e8a3e7ae34f3092eecc41b1a5a3d6755dcaccd2a66beec307849ef`.
The first normalized mismatch is byte 58,769 on page 8, where the bibliography
contains an extra left brace before `Stokes`. Exact base `62eede80a` and fixed
`.231` executables produce byte-identical DVI, `.aux`, and `.out` under the
same current format; their `.aux` and `.out` SHA-256 values are respectively
`fc8187a62d66973d7288246dca70c3dbe35ee805cb0702f1e12ebc6a18e2eb07`
and `a7c9a980b0351fbf9cf761e238b022a40e013b2b21bd1258f369f870303210aa`.
The historically matched format authority in `umber2-sdpz.230` remains the
DVI-exact evidence for this PDF series. The independent current-format defect
is `umber2-sdpz.233`, and it blocks `.232` before PDF work continues.
