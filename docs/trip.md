# Knuth TRIP Harness

Status: local-oracle-presence-conditional end-to-end conformance test.

The TeX82 TRIP and e-TeX V2 e-TRIP tests are pinned separately from the
external document corpus and share the same strict final-DVI oracle as Story
and Gentle. Run them with:

```bash
scripts/fetch-conformance-inputs.sh
scripts/fetch-conformance-inputs.sh --offline
cargo test -p umber --test it e2e_conformance_trip -- --nocapture
cargo test -p umber --test it e2e_conformance_etrip -- --nocapture
scripts/regen-fixtures.sh --case e2e/trip
scripts/regen-fixtures.sh --case e2e/etrip
scripts/setup-conformance-tests.sh
scripts/trip.sh
```

`scripts/trip.sh` is the canonical guarded entry point for hang and
memory-growth work. It defaults to a 120-second wall limit, 6144 MiB aggregate
RSS limit, 30-second output-progress limit, and five-second TERM grace, then
kills and reaps the complete process group. Override these with
`UMBER_TRIP_TIMEOUT_SECONDS`, `UMBER_TRIP_MAX_RSS_MIB`,
`UMBER_TRIP_PROGRESS_TIMEOUT_SECONDS`, and `UMBER_TRIP_TERM_GRACE_SECONDS`.
Arguments replace its default filtered TRIP/e-TRIP Cargo command. The helper selects the
finite default expansion-fuel budget explicitly. The format-loaded TRIP path
contains a deliberate nested `\message` construction at line 419: `\the` of
a token register must stay unexpanded while the complete message text is being
expanded, as in TeX82's `scan_toks(..., xpand=true)`. Expanding that replay a
second time recursively nests `\message{` and is an allocation-safety bug.
The same bounded replay keeps an engine-owned frozen sentinel below the raw
general text. Operand scanners may therefore read past a trailing value such
as `\the\count15` without exposing caller input, while the rendered digits are
still collected into the mark or definition. TRIP page 3 exercises this with
the numeric marks created by `\everypar`.

`scripts/fetch-conformance-inputs.sh` fetches official CTAN bytes into
gitignored `third_party/trip/` and verifies their SHA-256 hashes. The Cargo
integration test first checks for
`third_party/trip/trip.tex` and `trip.tfm`; when either is absent it returns
without running TRIP. When both are present it uses a shared in-process Rust
helper for format creation and the format-loaded run, then uses the shared
conformance library to compare against the gitignored,
locally generated `tests/corpus/e2e/trip.expected.dvi` oracle, requiring byte-identical final
DVI after normalizing only the preamble comment. DVItype is diagnostic for
Umber. Fixture regeneration independently executes both TRIP phases with
pdfTeX and installs that locally generated DVI through
`scripts/regen-fixtures.sh`; it never copies the official third-party DVI.
The official `tripin.log`, `trip.log`, `trip.fot`, and `tripos.tex` remain
pinned inputs for the future diagnostic transcript-parity tier; they do not
affect the current DVI gate.

## Source Pins

The source of record is the CTAN `systems/knuth/dist/tex` distribution. The
manifest uses the University of Utah CTAN mirror as a concrete acquisition URL
because the CTAN redirector can select mirrors with different availability.
The byte identity is pinned by SHA-256:

| File          | SHA-256                                                            |
| ------------- | ------------------------------------------------------------------ |
| `trip.tex`    | `15f15c2ca1470085299056ec89dea5f51e9fe9303ef25581b2f2eaf7809ae97b` |
| `trip.tfm`    | `2c94bdba9c769e885f357823a183aaa5d2267731075f040f2a03cf6442a26181` |
| `tripin.log`  | `ba01328756a8901d7c38162c9012014e9540322bf0963e105286f2a6ccb494cc` |
| `trip.log`    | `61a653523bdccab9fd3f9aa61d170d0198c322c951938327b7daef9b70f26d8b` |
| `trip.fot`    | `89e275ac12d025c06022e8dd6eb556b765954af2654b39ac2fbd451cf631b370` |
| `trip.typ`    | `64efc62b962c592c2973f8c45a78e9e5d473f8b9da53ee53bc275a98041675cc` |
| `trip.dvi`    | `09802695e330d34acec9192c15debe2de65e34fcbd3f947db9c8924240b1fe0a` |
| `tripos.tex`  | `ea7447c7a8f2de278d2f84474f22c48c9d8a0059d7e16edd578d0bbe7077b47f` |
| `tripman.tex` | `a3e47254ad87fc3fdba210d61764c93b021740f56465971f5a41103405add48b` |

The exact URLs live in `tests/trip-manifest.txt` beside the matching hashes.

The locally generated `tests/corpus/e2e/trip.expected.dvi` is not the official
`trip.dvi` above. It is generated locally from the pinned `trip.tex` and
`trip.tfm` by pdfTeX 3.141592653-2.6-1.40.27 (TeX Live 2025), using the
two-phase INITEX/format-loaded workflow. Its raw SHA-256 is
`a48cec413b485403e11d35e24122aa747b3e3863a151c257fcec026580a78bf9`;
after preamble-comment normalization it is
`6420f3461dec8e5feed4b03bfc3717d00c8a36fae4fe9226f6d53a4db7592bb9`.
Regenerate it with `scripts/regen-fixtures.sh --case e2e/trip`, setting
`UMBER_REF_PDFTEX` when pdfTeX is not on `PATH`.

## e-TRIP DVI Conformance

The same pinned manifest and acquisition path also fetch the official e-TeX
V2 e-TRIP sources and expected artifacts. The harness reuses `trip.tfm`
directly, as the e-TRIP manual states that `etrip.pl` is a copy of `trip.pl`.
`scripts/regen-fixtures.sh --case e2e/etrip` creates a renamed local e-TeX 2.6
adaptation of the official 2.0 source and generates the gitignored DVI oracle
with pdfTeX. The `e2e_conformance_etrip` Cargo test requires Umber to match
that DVI byte-for-byte after the standard preamble-comment normalization.
Official transcript, terminal-photo, DVItype, and output-file comparisons
remain part of the broader e-TRIP harness task.

The special reference engine comes from the TeX Live 2025 source snapshot
`texlive-20250308-source.tar.xz`, fetched from the University of Utah historic
archive and pinned by SHA-512
`0837c935488b96cfc8dd79f1298f283b467ab68b4163cee9cb04b79e80195982fdc5ae8a80058dc7d3e99206bfda8b3bdd11340425b08f60cbef70d5a0e22702`.
`tests/trip-reference-manifest.txt` additionally pins the extracted `tex.web`,
Web2C change inputs, TRIP configuration, and upstream TRIP harness by SHA-256.
The build records the exact configure/make commands and platform-specific tool
hashes in `target/trip-initex/build-record.txt`.

## Reference Toolchain

The current Cargo DVI gate does not require TeXware or Knuth's special TRIP
INITEX build. `scripts/build-trip-initex.sh` retains the hash-pinned Appendix A
toolchain for future transcript-parity work; it writes provenance and wrappers
under `target/trip-initex/`.

Umber's final-DVI oracle normalizes only the preamble comment and otherwise
requires byte identity. Any mismatch writes byte, page, opcode, and
disassembly context under `target/conformance-triage/trip/`.

## Umber format images

`umber run INPUT --format-out NAME.fmt` writes a format when INPUT terminates
with `\dump`; `umber run INPUT --format NAME.fmt` starts from that image. The
schema-10 format has an explicit fixed-width little-endian header and section
directory, compatibility fingerprints, deterministic alignment, and a
whole-image checksum. Its deterministic fixed sections contain semantic
engine state only: control-sequence namespaces and meanings, immutable
token/macro/glue/font and hyphenation content, code tables, environment cells,
and frozen node graphs. Loading validates and directly installs immutable
bases plus mutable job overlays; it never restores host pointers, hash-table
layout, allocation capacities, journals, checkpoints, input cursors,
provenance caches, or `World` effects. The official two-phase TRIP workload
exercises this format path before DVI comparison, while state tests instrument
the loader to reject regressions into graph remapping, semantic resealing, or
environment assignment replay. Schema 9 images are rejected
and regenerated from source; the durable container and frozen-store migration are specified in
[frozen_format.md](frozen_format.md).
