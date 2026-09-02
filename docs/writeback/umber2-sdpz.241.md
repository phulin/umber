# umber2-sdpz.241: canonical PDF text operators

The representative remains the complete unmodified recent-arXiv row
`2605.22212`, entrypoint and source-derived jobname `paper-JDE-o4`. Its
34,138-byte entrypoint has SHA-256
`d10e871e029a353935fe48f0ddd9119ea1dfb1b90a27875cd6fbc7298931c51c`.
The schema-8 distribution root aHash64 remains `cf7099ef97710816`, and Umber
loads the unchanged schema-12 pdfLaTeX object
`ahash64-v1-4225c474b34108a4`, SHA-256
`e02298271a57b081d055fa7c754a7d78f220d29618ca9c006e695c51cdb146af`.

pdftex.web section 690's `pdf_begin_string`, `pdf_set_text_pos`, and
`pdf_set_font` are one retained text-state machine. An unexpanded text matrix
starts and repositions with relative `Td`; a nonzero old or new auto-expansion
ratio requires absolute `Tm`. An unchanged font resource and TeX font size do
not repeat `Tf`. Adjacent characters and bounded kern or glue movements remain
inside one open `TJ` array until a font, position, literal, or graphics
boundary ends the string.

The detached PDF painter now retains the current font and text-line matrix and
buffers typed text and adjustment items until such a boundary. The mapped-text
cursor still uses the exact TeX anchors and pdfTeX width raster implemented by
the preceding parity work; the change only gives those items pdfTeX's
operator lifetime. A focused regression covers the exact unexpanded sequence
of one `Tf`, initial `Td`, and one `TJ` containing adjacent glyph and kern
runs. Existing controls retain `Tm` for auto-expanded fonts and prove that a
direct color operation preserves the text cursor and font selection.

The optimized test-profile Umber binary has SHA-256
`4ba591312b377774749552f6c140c8384433b3adfdc4ff2f0232d5b72db2dcc9`.
The single authenticated fresh-directory row used `SOURCE_DATE_EPOCH=1772323200`,
`FORCE_SOURCE_DATE=1`, offline mode, 500,000,000 expansion fuel, 10,000,000
execution steps, a 120-second wall guard, two-second termination grace, and
the authorized 2,048-MiB aggregate-RSS ceiling. Its 293,809-byte PDF has
SHA-256
`831687b0a25129671735c08f7d484b5d5ca542b2eeb6a4d89d3db5a67b574e7c`.

The normalized projection advances from
`f21b4ea9bfcc1a526d72199a6efc093f46d41a4a48ea7480e386df41a4203ec8`
to
`96e35c6dea8faca8356c08438e37980bbd1ba504e0dbe05d0f6b04f413aa60f5`;
the clean pdfTeX projection remains
`038c47236c87790cb5c18b008911fbc98a14eed943e1cd3209a9e2e920dcdbf5`.
Page 1 is exact through the opening font selection, `113.1 715.195 Td`, its
complete consolidated title `TJ`, and the following text and font transitions.
The next first difference is later on page 1: pdfTeX emits
`-219.916 -11.955 Td`, while Umber emits `-219.916 -11.956 Td`.

Page resources remain exact, including `/ProcSet [/PDF /Text]`. The F31, F32,
and F35 FontFile stream SHA-256 identities remain respectively
`f5cfbe11a9d7ad7ca22266ff08e1babd7014329a312574ce5523ab07deea0bae`,
`18a3eef3cdd18710e0f0d99ede535f6cab37a6639c1a8c84836284d13ac6331`,
and
`5cdd844ed607e5b416b33a09ff149ff4e7ea837f9013727063ef4ecb4538b8c7`;
F35 retains `/Flags 4 /ItalicAngle -14`. PDF-mode AUX and OUT remain exact at
`046f06e5c9423f147955408da40f62287acdb46ff329e8e1bd74ecd987b37ae9`
and
`a7c9a980b0351fbf9cf761e238b022a40e013b2b21bd1258f369f870303210aa`.
The loaded format is unchanged. This detached PDF paint change cannot affect
the preceding exact nine-page DVI, whose 64,380 bytes retain SHA-256
`d35269fbf6c2b4b5303b22194cde5de063d9af25e480f0c8a83636670eb0e96c`.

The focused `tex-out` suite passes all 176 tests. The next independent
coordinate-raster difference is tracked separately.
