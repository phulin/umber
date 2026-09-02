# umber2-sdpz.228: Type-1 descriptor fallback metrics

The PDF representative is locked recent-arXiv row `2605.22212`, the smallest
row already proven exact in normalized DVI. Its complete archive is 11,816
bytes with SHA-256
`6af819f487f2912c4b12f34fd5c4ba7e512626245d125a2deb00bf97f4f99a59`.
It contains two members with manifest SHA-256
`43a9ebb81ee5525a9946e21bab993774ce67f7b40007394952c781ef2f43c05c`.
The unmodified entrypoint is `paper-JDE-o4.tex`, 34,138 bytes with SHA-256
`d10e871e029a353935fe48f0ddd9119ea1dfb1b90a27875cd6fbc7298931c51c`,
and every run retained its source-derived jobname `paper-JDE-o4`.

The clean reference is pdfTeX 1.40.29 binary SHA-256
`608cb1760e9a471668ba97eea22fde60f2f7fadd285acd7c7b1ba243ddf71db3`,
build-record SHA-256
`232e68e4914c32d31a84e2c4d00a964189402388d1d80b776789195a5ffc54cd`,
paired pdfLaTeX format SHA-256
`d557a9b8d7de1ff9bc5a570830701417d32f09b300f0188f9a18a485aaf1ff8e`,
and format-receipt SHA-256
`bcaa10b08e9300ef1bc32fc22b834a29aca41e835e9d1d7e2090125bab04f4f8`.
It used the `texlive-20260301-texmf` runtime at aHash64
`1cd780f1ca7b3648`, authenticated by lock SHA-256
`836b9133624f2deb7a59de3159a66ca08b653635b300019fb49179e6dae30621`.
With `SOURCE_DATE_EPOCH=1772323200`, `FORCE_SOURCE_DATE=1`, and the
120-second/1536-MiB/2-second guards, clean DVI and PDF runs both exited zero.
The DVI is 9 pages and 64,380 bytes with SHA-256
`9e709b574eb25988b594e7b980b1b728f86be3220df3d6a6676e1acb9f20d8ee`;
the PDF is 9 pages and 284,610 bytes with SHA-256
`dd70d6f6633cdab874b117f1e428f45a054a69d6ed22e2faa68341c34877fc24`.

The fixed Umber binary has SHA-256
`0fcd4c6da5df607ad3e5edddea1773f24c9ede666c54f6725506d9268098002b`.
It used schema-12 format object SHA-256
`e02298271a57b081d055fa7c754a7d78f220d29618ca9c006e695c51cdb146af`
from authenticated distribution manifest SHA-256
`2595bf2f5d98f2613ef4456a2f2e3eb1f75c9c4eb324338cafa62a696f5ed20a`
at root aHash64 `49dd828f7839a1c3`. Offline DVI and PDF runs both
exited zero under 500,000,000 expansion fuel, 10,000,000 execution steps,
and the same clock and external guards. The Umber DVI is 9 pages and 64,380
bytes with raw SHA-256
`d35269fbf6c2b4b5303b22194cde5de063d9af25e480f0c8a83636670eb0e96c`;
`parity-harness --compare-existing-dvi` accepts all nine pages after its sole
preamble-comment normalization. DVI-mode auxiliary bytes are exact at SHA-256
`fc8187a62d66973d7288246dca70c3dbe35ee805cb0702f1e12ebc6a18e2eb07`,
and the `.out` bytes are exact at SHA-256
`a7c9a980b0351fbf9cf761e238b022a40e013b2b21bd1258f369f870303210aa`.

The first Hayro-normalized PDF difference was page 1's first Type-1 font
descriptor. Clean pdfTeX emitted `/Ascent 694` and `/Descent -194`; Umber
emitted `/Ascent 750` and `/Descent -250`. pdftex.web §799 delegates final
font emission to `do_pdf_font` and `write_fontstuff`. The pinned 1.40.29
`writefont.c::preset_fontmetrics` defines missing Type-1 metrics from TFM
character `h` for ascent and character `y` for descent, with `H` and the TFM
x-height owning the other fallbacks. Umber instead selected the maximum height
and depth across all 256 codes. The finalizer now implements the named-character
rule, and a focused negative control proves that taller and deeper unrelated
characters do not change these descriptor values.

The focused `embedded_type1` closed fixture now carries Umber PDF SHA-256
`06602729c9c9e906a0f8b2a9ce571dc59284e5b1eb6015afa817aa2f82301c00`
and normalized-structure SHA-256
`198d72b3030f21219fdad5309ebebd21e6b39d76720eec0585e0dc45c985868e`.
Its descriptor changed only from `/Ascent 750 /Descent -250` to
`/Ascent 694 /Descent -194`; the embedded program, rendered pixels, extracted
text, and clean-reference identities remain unchanged.

The fixed PDF is 341,517 bytes with SHA-256
`7a360e67d9286abd685faf1fd33f4af3f7cf0f5867c5b4355d0c269138ce0580`.
Its normalized projection advances past the fixed descriptor metrics and has
SHA-256
`4a7f34e1d467900ab853b4e771d7d8e518bb8b72a8587779b4d5546899ef0620`.
The next first difference is the same font's independently differing embedded
FontFile stream; its observed identities and reproduction are recorded without
diagnosis in successor `umber2-sdpz.229`.
