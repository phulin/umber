# umber2-sdpz.239: canonical Type-1 `/ItalicAngle`

The representative remains the complete unmodified recent-arXiv row
`2605.22212`, entrypoint and source-derived jobname `paper-JDE-o4`. Its
34,138-byte entrypoint has SHA-256
`d10e871e029a353935fe48f0ddd9119ea1dfb1b90a27875cd6fbc7298931c51c`.
The schema-8 distribution manifest has SHA-256
`6ae7fb0a5ae6b7901c55e804cce07d2c353713f3ca8207d84c6ea1e64a3bbb5a`
and root aHash64 `cf7099ef97710816`. Umber loads the unchanged 1,126,714-byte
schema-12 pdfLaTeX object at `ahash64-v1-4225c474b34108a4`, SHA-256
`e02298271a57b081d055fa7c754a7d78f220d29618ca9c006e695c51cdb146af`.

pdftex.web section 799 delegates embedded Type-1 font construction to
`write_fontstuff`. Pinned `writet1.c::t1_scan_keys` parses program metrics
through `t1_scan_num`, whose `%g` conversion produces a single-precision
floating-point value. Assignment to the integer descriptor slot truncates the
value toward zero. CMMI10 declares `/ItalicAngle -14.04`; pdfTeX therefore
emits `/ItalicAngle -14`, while Umber's integer-only parser rejected the
decimal and emitted its zero fallback.

`PdfType1Program` now parses `/ItalicAngle` as a finite `f32` and truncates it
toward zero before detached PDF finalization. `write_fontdescriptor` separately
uses the embedded-program `/Flags 4` default rather than deriving italic or
fixed-pitch bits from program metrics. Focused controls cover negative and
positive fractional values plus a nonnumeric value. The change only reads the
already selected program and does not mutate Type-1 subset bytes.

The optimized test-profile Umber binary has SHA-256
`7d000a273c0239079150b9638b103723dccc3e32f95781804e16c291b6509f4d`.
The authenticated fresh row used `SOURCE_DATE_EPOCH=1772323200`,
`FORCE_SOURCE_DATE=1`, 500,000,000 expansion fuel, 10,000,000 execution steps,
a 120-second wall guard, two-second termination grace, and the authorized
2,048-MiB aggregate-RSS ceiling. Its 298,252-byte PDF has SHA-256
`d953a2d0d3554bee0c04911945fc85a76abfe60b5156e53262b96c6dda05aa81`.
F35 now has canonical `/Flags 4 /ItalicAngle -14`; its FontFile remains exact
at 15,664 bytes with SHA-256
`5cdd844ed607e5b416b33a09ff149ff4e7ea837f9013727063ef4ecb4538b8c7`.

F31 remains exact at 7,948 FontFile bytes with SHA-256
`f5cfbe11a9d7ad7ca22266ff08e1babd7014329a312574ce5523ab07deea0bae`.
F32 remains exact at 25,245 raw FontFile bytes, `/Length2 23712`, and SHA-256
`18a3eef3cdd18710e0f0d99ede535f6cab37a6639c1a8c84836284d13ac6331`.
The PDF-mode AUX and OUT remain byte-exact at
`046f06e5c9423f147955408da40f62287acdb46ff329e8e1bd74ecd987b37ae9`
and
`a7c9a980b0351fbf9cf761e238b022a40e013b2b21bd1258f369f870303210aa`.
The loaded format remains the unchanged object at SHA-256
`e02298271a57b081d055fa7c754a7d78f220d29618ca9c006e695c51cdb146af`;
the pure detached descriptor fix does not affect the preceding exact
nine-page DVI, whose raw 64,380-byte Umber artifact has SHA-256
`d35269fbf6c2b4b5303b22194cde5de063d9af25e480f0c8a83636670eb0e96c`.

The normalized PDF advances from
`a11d7a312565651424cc93ab1bb4d08b6d5af98349387e58f3afae02d9555342`
to
`f014d0f54fca1ba1b3a0afddfbb1029e426e43d3d52ce7bd38758ce8d7fcc429`;
the clean projection remains
`038c47236c87790cb5c18b008911fbc98a14eed943e1cd3209a9e2e920dcdbf5`.
The first remaining independent difference is page 1's `/ProcSet`: clean
pdfTeX emits `[/PDF /Text]`, while Umber emits `[/PDF]`. Its successor is
tracked separately as `umber2-sdpz.240`.

The focused `tex-fonts` and `tex-out` suites pass 86 and 174 tests, and the
full `cargo test -q --tests` routine suite passes. The single
`scripts/check.sh` run reports all four gates passed.
