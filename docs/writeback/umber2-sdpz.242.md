# umber2-sdpz.242: canonical relative text-coordinate rounding

The representative remains complete unmodified recent-arXiv row
`2605.22212`, entrypoint and source-derived jobname `paper-JDE-o4`. Its
34,138-byte entrypoint has SHA-256
`d10e871e029a353935fe48f0ddd9119ea1dfb1b90a27875cd6fbc7298931c51c`.
The schema-8 distribution root aHash64 remains `cf7099ef97710816`, and Umber
loads the unchanged schema-12 pdfLaTeX object
`ahash64-v1-4225c474b34108a4`, SHA-256
`e02298271a57b081d055fa7c754a7d78f220d29618ca9c006e695c51cdb146af`.

The issue description had the two operands reversed. The authenticated clean
pdfTeX PDF has raw SHA-256
`dd70d6f6633cdab874b117f1e428f45a054a69d6ed22e2faa68341c34877fc24`
and normalized SHA-256
`038c47236c87790cb5c18b008911fbc98a14eed943e1cd3209a9e2e920dcdbf5`;
it emits `-219.916 -11.956 Td`. The recorded pre-fix Umber PDF has raw SHA-256
`831687b0a25129671735c08f7d484b5d5ca542b2eeb6a4d89d3db5a67b574e7c`
and normalized SHA-256
`96e35c6dea8faca8356c08438e37980bbd1ba504e0dbe05d0f6b04f413aa60f5`;
it emits `-219.916 -11.955 Td`.

pdftex.web section 690's `pdf_begin_string` calculates a relative vertical
movement from the exact scaled `pdf_v - cur_v`. Its `divide_scaled` call rounds
that delta to `fixed_decimal_digits` and returns `scaled_out`, which
`pdf_set_text_pos` uses to advance the retained scaled `pdf_v`. The same rule
applies horizontally from `cur_h - pdf_tj_start_h`; an absolute `Tm` instead
establishes both retained positions from their independently rounded absolute
coordinates. Subtracting two already-rounded floating-point positions is not
equivalent and caused the one-unit vertical difference.

Detached text runs now carry exact horizontal and vertical scaled coordinates
separately from mapped-font width-raster state. The paint interpreter forms
`Td` deltas from those coordinates, rounds them through the pinned
`divide_scaled` algorithm, retains its `scaled_out` positions, and converts to
floating point only for the final typed writer call. The focused negative
control uses raw positions 47,000,016 and 46,213,562: their independently
rounded values are 714.484 and 702.529, whose difference is -11.955, while the
canonical raw delta serializes as -11.956.

The matched-row optimized Umber binary has SHA-256
`2f2f4bd0346855f879eb3b05c58129b482bef60fe017e11310c11645e9ec1631`.
The single authenticated fresh-directory row used `SOURCE_DATE_EPOCH=1772323200`,
`FORCE_SOURCE_DATE=1`, offline mode, 500,000,000 expansion fuel, 10,000,000
execution steps, a 120-second wall guard, two-second termination grace, and
the authorized 2,048-MiB aggregate-RSS ceiling. Its PDF has SHA-256
`66273e224c6ffcae10c041e95034032a1e880b3b14b30ca355aa2992ee010093`;
the normalized projection has SHA-256
`0c9ed781991049566b030f8e610be60ae260b33b55eb31b36ccd6243f07a768d`.
The projection is exact through all page-1 and page-2 output and page 3's text
through the final `[(.)] TJ` and `ET` before the first rule.

Page resources and the F31, F32, and F35 programs and descriptors remain
exact. Their FontFile stream SHA-256 identities remain respectively
`f5cfbe11a9d7ad7ca22266ff08e1babd7014329a312574ce5523ab07deea0bae`,
`18a3eef3cdd18710e0f0d99ede535f6cab37a6639c1a8c84836284d13ac6331`,
and
`5cdd844ed607e5b416b33a09ff149ff4e7ea837f9013727063ef4ecb4538b8c7`;
F35 retains `/Flags 4 /ItalicAngle -14`. AUX and OUT remain byte-exact at
`046f06e5c9423f147955408da40f62287acdb46ff329e8e1bd74ecd987b37ae9`
and
`a7c9a980b0351fbf9cf761e238b022a40e013b2b21bd1258f369f870303210aa`.
The loaded format and downstream-independent exact nine-page DVI remain
unchanged; the DVI's prior SHA-256 is
`d35269fbf6c2b4b5303b22194cde5de063d9af25e480f0c8a83636670eb0e96c`.

The focused `tex-out` suite passes all 177 tests. The nine committed embedded
font controls were stale after the preceding text-operator fix; their Umber
PDF, normalized-structure, and render-attestation channels now pin the current
`Td`/`TJ` spelling, with unchanged reference PDFs, rasters, and extracted
text. Their focused parity test and the complete `cargo test -q --tests` suite
pass. The final `scripts/check.sh` verdict is `all 4 gates passed`.

The next independent page-3 rule-paint difference is tracked separately as
`umber2-sdpz.243`.
