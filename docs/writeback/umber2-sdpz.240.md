# umber2-sdpz.240: canonical page `/ProcSet` classes

The representative remains the complete unmodified recent-arXiv row
`2605.22212`, entrypoint and source-derived jobname `paper-JDE-o4`. Its
34,138-byte entrypoint has SHA-256
`d10e871e029a353935fe48f0ddd9119ea1dfb1b90a27875cd6fbc7298931c51c`.
The schema-8 distribution manifest has SHA-256
`6ae7fb0a5ae6b7901c55e804cce07d2c353713f3ca8207d84c6ea1e64a3bbb5a`
and root aHash64 `cf7099ef97710816`. Umber loads the unchanged 1,126,714-byte
schema-12 pdfLaTeX object at `ahash64-v1-4225c474b34108a4`, SHA-256
`e02298271a57b081d055fa7c754a7d78f220d29618ca9c006e695c51cdb146af`.

pdftex.web sections 766--768 derive `/ProcSet` from each page or form's
generated resource lists. `/PDF` is unconditional. A nonempty font list adds
`/Text`; direct raster images union the `writeimg.c` color mask into
`/ImageB`, `/ImageC`, and `/ImageI` in that order. `writepng.c` assigns both
the color and indexed bits to a palette image. Empty pages, ordinary graphics,
nested forms, and imported PDF pages add no class. The existing omission
predicate from section 768 remains unchanged.

Detached finalization now carries one page-local resource classifier while it
discovers fonts and directly referenced images, and derives form `/Text` from
the form-local font list. A focused table covers empty, graphics-only,
text-only, grayscale, color, indexed, imported-PDF, and mixed resources. The
15-case PDF fixture cohort proves that all nine text-bearing outputs acquire
`/Text`, while empty, graphics-only, form-only, and imported-PDF cases remain
byte-identical. Exact Poppler 25.08.0 rendering and extraction remain unchanged.

The optimized test-profile Umber binary has SHA-256
`77b565ac5bef4e61b92ed7aaa9ee6d578b3ea97991fe8b2ad548d4ee8ee4385b`.
The authenticated fresh row used `SOURCE_DATE_EPOCH=1772323200`,
`FORCE_SOURCE_DATE=1`, offline mode, 500,000,000 expansion fuel, 10,000,000
execution steps, a 120-second wall guard, two-second termination grace, and
the authorized 2,048-MiB aggregate-RSS ceiling. Its 298,259-byte PDF has
SHA-256
`94dafd0b7d2b366e2992eb641db9a0370d90c6475d9c0beeb549c4724d01f606`.

The clean and Umber normalized projections are now byte-exact through page
1's resource dictionary, including `/ProcSet [/PDF /Text]`. Consequently F31,
F32, and F35 remain exact: their FontFile streams are respectively 7,948,
25,245, and 15,664 bytes with SHA-256
`f5cfbe11a9d7ad7ca22266ff08e1babd7014329a312574ce5523ab07deea0bae`,
`18a3eef3cdd18710e0f0d99ede535f6cab37a6639c1a8c84836284d13ac6331`,
and
`5cdd844ed607e5b416b33a09ff149ff4e7ea837f9013727063ef4ecb4538b8c7`.
F35 retains `/Flags 4 /ItalicAngle -14`.

The PDF-mode AUX and OUT remain byte-exact at
`046f06e5c9423f147955408da40f62287acdb46ff329e8e1bd74ecd987b37ae9`
and
`a7c9a980b0351fbf9cf761e238b022a40e013b2b21bd1258f369f870303210aa`.
The loaded format remains the same object identified above. This pure detached
PDF-resource change cannot affect DVI lowering; the preceding exact nine-page,
64,380-byte DVI remains at SHA-256
`d35269fbf6c2b4b5303b22194cde5de063d9af25e480f0c8a83636670eb0e96c`.

The normalized PDF advances from
`f014d0f54fca1ba1b3a0afddfbb1029e426e43d3d52ce7bd38758ce8d7fcc429`
to
`f21b4ea9bfcc1a526d72199a6efc093f46d41a4a48ea7480e386df41a4203ec8`;
the clean projection remains
`038c47236c87790cb5c18b008911fbc98a14eed943e1cd3209a9e2e920dcdbf5`.
The first remaining independent difference is page 1's initial positioned
text: clean pdfTeX uses `Td` followed by one consolidated `TJ`, while Umber
uses `Tm`, a leading `Tj`, and repeated font selections with split `TJ`
operations. That successor is tracked as `umber2-sdpz.241`.

The focused `tex-out` suite passes 175 tests and the six-test hermetic PDF
parity cohort passes. The single `scripts/check.sh` run reports all four gates
passed.
