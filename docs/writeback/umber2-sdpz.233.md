# umber2-sdpz.233: matched current pdfLaTeX formats

The page-8 brace was caused by a stale Umber format, not by current command
semantics. The current schema-8 snapshot had root aHash64
`df66c327ae636145` and manifest SHA-256
`a68acebc1a83fd4ec0ce8c3baed4e8fe01de9b37e5a878b2f1fc203c2f20662f`,
but embedded the 1,139,703-byte schema-12 pdfLaTeX object with SHA-256
`ddd082db722654fd9c47e1080d8e128d1a5b0cfb3186f9e3bc01f8943f559ad4`.
The format source lock still named the retired schema-6 construction root
`7d0fdcf5b35d0058`; current Umber rejected that root before format execution.
The lock now names the current schema-8 authority, and its SHA-256 is
`36c1cfa6b7adc98727f0186a43a629463084640c955064da148a9dd24ea645d6`.

The official snapshot action used a fresh issue-local format cache, the exact
locked TeX Live 2026 `texmf-dist`, 2,000,000,000 format-construction fuel, and
the 2-GiB RSS guard. It constructed pdfLaTeX once. The resulting schema-12
image is 1,126,714 bytes, has aHash64 `4225c474b34108a4`, and has SHA-256
`e02298271a57b081d055fa7c754a7d78f220d29618ca9c006e695c51cdb146af`.
That SHA-256 is exactly the historical matched-format authority used before
the stale snapshot object appeared.

The issue-local distribution is
`target/umber2-sdpz.233/distribution`. Its schema-8 manifest is 80,935 bytes,
has root aHash64 `cf7099ef97710816`, and has SHA-256
`6ae7fb0a5ae6b7901c55e804cce07d2c353713f3ca8207d84c6ea1e64a3bbb5a`.
Its pdfLaTeX record names the exact object above, format schema 12, source
distribution `texlive-20260301-texmf`, source-manifest aHash64
`df66c327ae636145`, fixed epoch `1772323200`, and the complete 64-key input
closure.

The clean pdfTeX 1.40.29 binary has SHA-256
`608cb1760e9a471668ba97eea22fde60f2f7fadd285acd7c7b1ba243ddf71db3`.
Its source manifest has SHA-256
`34e52e80adf523fa5442de8232ea5a48ab71ca9e3938365f81f31fba76f0a93d`.
One recorder-audited INITEX run produced the 2,232,923-byte clean pdfLaTeX
format at
`target/umber2-sdpz.233/reference/pdftex14029-reference-format/pdflatex.fmt`.
Its SHA-256 is
`d557a9b8d7de1ff9bc5a570830701417d32f09b300f0188f9a18a485aaf1ff8e`.
The lock's source records did not change when the distribution authority was
advanced; the receipt's distribution and complete-lock identities were
resealed without a second engine run. The resulting receipt has SHA-256
`34fcfb5bedb73a59c1dddf1090138d6efbbb23d4727e7fedca2667ed9103f482`.

The repository format-pair gate passed against those exact artifacts. Its
receipt is `target/umber2-sdpz.233/format-pair/pairing.json`, with SHA-256
`49adef7a8b282d789423ace991d1922f017778327c87e3a7c7d75248d98f0411`.
The shared-counter/`amsthm` markers have SHA-256
`982ce10d0f38f8fb72d0cba432b4fa1aeea481b0a6cf0163f9c7fe0bb5183147`,
and the probe's normalized DVI has SHA-256
`4b5bafe8a7682b2ba935f4bee121fa94178307f1c069121dceb36afce22be258`.

The complete unmodified recent-arXiv row remains `2605.22212`, entrypoint and
source-derived jobname `paper-JDE-o4`. The 11,816-byte archive has SHA-256
`6af819f487f2912c4b12f34fd5c4ba7e512626245d125a2deb00bf97f4f99a59`;
the 34,138-byte entrypoint has SHA-256
`d10e871e029a353935fe48f0ddd9119ea1dfb1b90a27875cd6fbc7298931c51c`,
and the two-member manifest has SHA-256
`43a9ebb81ee5525a9946e21bab993774ce67f7b40007394952c781ef2f43c05c`.
Each engine ran the complete materialized source exactly once, offline, with
the source-derived side files, fixed epoch, 120-second/1,536-MiB guard, and
two-second termination grace. Umber additionally used 500,000,000 expansion
fuel and 10,000,000 execution steps; its executable has SHA-256
`801e3be1ceb89090c8b010b5dab05980da87f7fe88115f2525ca35023e5dbde6`.

Both engines produced nine-page, 64,380-byte DVI. The raw clean-pdfTeX DVI has
SHA-256
`9e709b574eb25988b594e7b980b1b728f86be3220df3d6a6676e1acb9f20d8ee`;
the raw Umber DVI has SHA-256
`d35269fbf6c2b4b5303b22194cde5de063d9af25e480f0c8a83636670eb0e96c`.
After the parity harness's sole preamble-comment normalization, every byte is
exact; the normalized SHA-256 is
`732afeca3dc3ee599a8b7357b5896f5ce1f7c6143608ecc1dd41ee2e85a29f6a`.
The 5,069-byte AUX files are exact at SHA-256
`fc8187a62d66973d7288246dca70c3dbe35ee805cb0702f1e12ebc6a18e2eb07`,
and the 3,242-byte OUT files are exact at SHA-256
`a7c9a980b0351fbf9cf761e238b022a40e013b2b21bd1258f369f870303210aa`.
No paper-specific or engine-semantic change was required. This restores the
DVI precondition for `umber2-sdpz.232` to continue the independent PDF
Type-1 trailer comparison.
