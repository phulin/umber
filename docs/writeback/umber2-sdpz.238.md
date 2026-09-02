# umber2-sdpz.238: Type-1 binary/text line boundaries

The representative remains complete unmodified recent-arXiv row
`2605.22212`, entrypoint and source-derived jobname `paper-JDE-o4`. Its
34,138-byte entrypoint has SHA-256
`d10e871e029a353935fe48f0ddd9119ea1dfb1b90a27875cd6fbc7298931c51c`.
The schema-8 distribution manifest has SHA-256
`6ae7fb0a5ae6b7901c55e804cce07d2c353713f3ca8207d84c6ea1e64a3bbb5a`
and root aHash64 `cf7099ef97710816`. Umber loaded the unchanged 1,126,714-byte
schema-12 pdfLaTeX object at `ahash64-v1-4225c474b34108a4`, SHA-256
`e02298271a57b081d055fa7c754a7d78f220d29618ca9c006e695c51cdb146af`.

Before this fix, F32's outer-decrypted eexec bytes first differed at one-based
byte 21,350. The preceding selected `/o` CharString ends in the binary byte
`0x20`. pdfTeX emitted that byte followed directly by `ND\n`, while Umber
copied the source separator and emitted `0x20␣ND\n`. The extra byte changed
all following eexec ciphertext and made Umber's decoded program 26,424 bytes
with `/Length2 23713` instead of 26,423 bytes with `/Length2 23712`.

pdftex.web section 799 delegates embedded Type-1 writing to `write_fontstuff`.
Pinned `writet1.c::t1_getline` passes text through
`ptexmac.h::append_char_to_buf`, but copies the declared CharString bytes
directly into the same line buffer. The first textual byte after that binary
span is therefore normalized against the last binary byte. When both are an
ASCII space, the suffix separator is suppressed. `cs_store` retains that
mixed binary/text tail, and `t1_flush_cs` writes it after reconstructing the
glyph name and declared byte count.

The Type-1 subset owner now reconstructs every retained Subr and CharString in
that same shape. It keeps the binary program exact, normalizes only the
following textual suffix with the binary final byte as prior line-buffer
state, and preserves the normalized header newline separately. A synthetic
control uses one valid CharString whose encrypted final byte is `0x20` and a
second whose final byte is `0x1e`: the former must suppress the separator and
the latter must retain it. The committed CMR10 `A`/`B`/`C` subset remains
byte-exact at lengths `1391/7337/0` and MD5
`fce9d1c28cd155a89a22e437b3c33f91`, proving the rule does not remove an
ordinary boundary.

The optimized test-profile Umber binary has SHA-256
`d32dd1fc026576e784b318adb8e67f5cd00806fea577104d25e22e0ebf154537`.
The authenticated fresh first-pass row used `SOURCE_DATE_EPOCH=1772323200`,
`FORCE_SOURCE_DATE=1`, 500,000,000 expansion fuel, 10,000,000 execution
steps, a 120-second wall guard, two-second termination grace, and the
authorized 2,048-MiB aggregate-RSS ceiling. Its 298,239-byte PDF has SHA-256
`691d56e3414731b8d0465af0b5648d48ab85198a72353ababeb16788dea7a711`.

F32 is now byte-exact with clean pdfTeX: its 25,245-byte raw Flate stream has
SHA-256
`18a3eef3cdd18710de0f0d99ede535f6cab37a6639c1a8c84836284d13ac6331`,
and its 26,423-byte decoded Type-1 program has SHA-256
`3b1aa560297b9468897ccce869f15250df12ffbc1b0a1d5370684dc8588b4ab7`
with `/Length1 2711 /Length2 23712 /Length3 0`. F31 remains exact: its
FontFile is 7,948 bytes with SHA-256
`f5cfbe11a9d7ad7ca22266ff08e1babd7014329a312574ce5523ab07deea0bae`,
and its ToUnicode stream is 740 bytes with SHA-256
`747613b0b34a24c7b1cdc3f60b3ca882bf212c097ca9c25b11a7176536503fb5`.

The normalized PDF advances from
`3f205c4d2f4a9b242047ca655a17c2e2b51174563b39f8a3ccc21b95ad537ad1`
to
`a11d7a312565651424cc93ab1bb4d08b6d5af98349387e58f3afae02d9555342`;
the clean projection remains
`038c47236c87790cb5c18b008911fbc98a14eed943e1cd3209a9e2e920dcdbf5`.
PDF-mode AUX and OUT remain exact at
`046f06e5c9423f147955408da40f62287acdb46ff329e8e1bd74ecd987b37ae9`
and
`a7c9a980b0351fbf9cf761e238b022a40e013b2b21bd1258f369f870303210aa`.
The pure post-execution font-program transformation does not alter the loaded
format or DVI construction; the preceding exact nine-page normalized DVI and
its raw 64,380-byte Umber artifact remain unchanged.

The first remaining normalized difference is F35's descriptor
`/ItalicAngle`: clean pdfTeX emits `-14`, while Umber emits `0`. Its FontFile
stream is already exact at 15,664 bytes with SHA-256
`5cdd844ed607e5b416b33a09ff149ff4e7ea837f9013727063ef4ecb4538b8c7`.
That independent successor is tracked as `umber2-sdpz.239`.
