# Knuth TRIP Harness

Status: local-oracle-presence-conditional end-to-end conformance test.

The TeX82 TRIP and e-TeX V2 e-TRIP tests are pinned separately from the
external document corpus and share the same strict final-DVI oracle as Story
and Gentle. Run them with:

```bash
scripts/trip.sh
scripts/trip.sh --offline
scripts/trip.sh self-test
scripts/build-trip-initex.sh
cargo test -p umber --test it e2e_conformance_trip -- --nocapture
cargo test -p umber --test it e2e_conformance_etrip -- --nocapture
scripts/regen-fixtures.sh --case e2e/trip
scripts/regen-fixtures.sh --case e2e/etrip
scripts/setup-conformance-tests.sh
```

`scripts/trip.sh` fetches official CTAN bytes into gitignored
`third_party/trip/`, verifies SHA-256 hashes, uses the pinned official
`trip.tfm`, runs the INITEX transcript phase, runs the format-loaded TRIP
phase, and runs DVItype. The Cargo integration test first checks for
`third_party/trip/trip.tex` and `trip.tfm`; when either is absent it returns
without running TRIP. When both are present it uses a shared in-process Rust
helper for format creation and the format-loaded run, then uses the shared
conformance library to compare against the gitignored,
locally generated `tests/corpus/e2e/trip.expected.dvi` oracle, requiring byte-identical final
DVI after normalizing only the preamble comment. DVItype is diagnostic for
Umber. Fixture regeneration independently executes both TRIP phases with
pdfTeX and installs that locally generated DVI through
`scripts/regen-fixtures.sh`; it never copies the official third-party DVI.
The generated `tripin.log`, `trip.log`, `trip.fot`, and `tripos.tex` remain in
`target/trip/` for diagnosis, but their parity belongs to the diagnostic tier
and does not affect this DVI milestone. The self-test does not fetch or run
TeX. It uses deterministic synthetic DVI and DVItype streams to prove both
signs of the special-reference phase's exact 64sp reconciliation boundary,
reject 65sp and structural or semantic changes across the DVI command classes,
and verify that each
rejection reports byte, page, opcode, and surrounding context as appropriate.
It also seeds retained format, DVI, and DVItype outputs and proves that failing
producers cannot reuse them.

`--keep-work` retains diagnostic transcripts and diffs, but never a generated
artifact as proof of a later invocation. Each Appendix A producer starts with
its format, DVI, or DVItype output absent, handles the intentional TRIP engine
exit status explicitly, and must create a fresh nonempty artifact. DVItype
must additionally exit successfully in the standalone reference workflow.
Thus a failed INITEX,
format-loaded run, or DVItype conversion cannot be hidden by an earlier green
run in the same `target/trip/` directory.

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

## Required Tools

The standalone DVItype phase requires `dvitype`, overridable with
`UMBER_REF_DVITYPE`. The Cargo integration test does not require TeXware.

The reference execution phase requires Knuth's special TRIP INITEX build from
`tripman.tex` Appendix A. In particular, that build sets
`mem_min=mem_bot=1`, `mem_top=mem_max=3000`, `error_line=64`,
`half_error_line=32`, and `max_print_line=72`, with statistics enabled. Select
it with:

```bash
UMBER_TRIP_INITEX=/absolute/path/to/special-initex scripts/trip.sh
```

Stock `pdftex -ini` or `tex -ini` is intentionally not accepted as a passing
oracle; it produces different line wrapping, banners, and capacity statistics.

Build and run the pinned tools with:

```bash
scripts/build-trip-initex.sh
scripts/build-trip-initex.sh --offline # verifies/reuses the cached archive
scripts/trip.sh reference --offline
```

The harness automatically finds `target/trip-initex/bin`; an alternate build
can be selected with `UMBER_TRIP_TOOLS`. The wrappers set `LC_ALL=C`,
`LANGUAGE=C`, and `TEXMFCNF` to the hash-pinned upstream
`triptrap/texmf.cnf`. That file establishes Appendix A's memory and line
settings and the normal TeX82 capacities that Web2C permits at runtime.

## Normalization

Appendix A item 6 permits slight floating-point differences, alternative
`right`/`w`/`x` and `down`/`y`/`z` encodings, and characters, rules, and
specials in almost the same positions. It does not specify a numeric movement
tolerance. Umber makes that permission executable with a deliberately
conservative project policy: corresponding movement operands may differ by at
most 64 scaled points under the structural checks below. The numeric bound is
Umber's policy, not a threshold quoted from Knuth.

The standalone special-reference validation applies only these normalizations:

- `trip.typ`: DVItype packaging text on line 1 and the quoted rendering of the
  DVI preamble comment. Movement operands may be reconciled only when opcode,
  byte offset, and all surrounding text match and the delta is at most 64
  scaled points.
- `trip.dvi`: DVI preamble comment bytes only, preserving the original comment
  length. A structured DVI walk additionally permits at most 64 scaled points
  of variation in movement operands while requiring identical opcode/operand
  structure and exact identity for characters, rules, specials, fonts, page
  structure, dimensions, and every non-movement operand.

These Appendix A allowances validate the pinned reference toolchain only. The
Umber end-to-end oracle does not apply movement reconciliation: after the DVI
preamble comment is normalized, every byte must match the official
`trip.dvi`. Any mismatch writes strict byte/page/opcode and disassembly context
under `target/conformance-triage/trip/`. `scripts/trip.sh
self-test` exercises the allowance boundary and adversarially changes movement
encoding, characters, rules, specials, fonts, page
structure/pointers/dimensions, and non-movement operands. Parallel DVItype
cases prove that opcode, byte offset, and surrounding text must remain exact.
Every rejected DVI case must retain actionable byte/page/opcode context.
Diagnostic text normalization and parity are owned separately and are
intentionally absent from this harness.

## Current Divergence Policy

The harness must not normalize or bless semantic Umber failures. A DVI or
DVItype mismatch should be investigated against TeX82's `ship_out`,
`hlist_out`/`vlist_out`, and `movement` procedures in `tex.web`; linked engine
work belongs under `umber2-i8w`, not in special cases in this script. The
committed-artifact and downstream DVI boundary is documented in
[architecture.md §10, "Output drivers (`tex-out`)"].

[architecture.md §10, "Output drivers (`tex-out`)"]: architecture.md#10-output-drivers-tex-out

## Umber format images

`umber run INPUT --format-out NAME.fmt` writes a format when INPUT terminates
with `\dump`; `umber run INPUT --format NAME.fmt` starts from that image. The
format has an explicit magic/version header, payload length, interaction mode,
and checksum. Its deterministic payload contains semantic engine state only:
control-sequence namespaces and meanings, immutable token/macro/glue/font and
hyphenation content, code tables, and environment cells. Loading validates and
rebuilds fresh dense stores; it never restores host pointers, hash-table
layout, allocation capacities, journals, checkpoints, input cursors,
provenance caches, or `World` effects. Logical node graphs such as box
registers remap into a fresh arena rather than preserving process-local arena
identities. The official two-phase TRIP workload exercises this format path
before DVI comparison.
