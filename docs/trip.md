# Knuth TRIP Harness

Status: explicit conformance gate, outside `cargo test --workspace --tests`.

The original TeX82 TRIP test is pinned separately from the external document
corpus and from any later e-TRIP work. Run it with:

```bash
scripts/trip.sh
scripts/trip.sh --offline
scripts/trip.sh self-test
```

`scripts/trip.sh` fetches official CTAN bytes into gitignored
`third_party/trip/`, verifies SHA-256 hashes, rebuilds `trip.tfm` through
PLtoTF and TFtoPL, runs the INITEX transcript phase, runs the format-loaded
TRIP phase, runs DVItype, and compares the official log/photo/DVItype/DVI and
`tripos.tex` artifacts. The self-test does not fetch or run TeX; it perturbs a
copied text artifact and verifies that `target/trip/diffs/` receives an
actionable unified diff.

## Source Pins

The source of record is the CTAN `systems/knuth/dist/tex` distribution. The
manifest uses the University of Utah CTAN mirror as a concrete acquisition URL
because the CTAN redirector can select mirrors with different availability.
The byte identity is pinned by SHA-256:

| File | SHA-256 |
| --- | --- |
| `trip.tex` | `15f15c2ca1470085299056ec89dea5f51e9fe9303ef25581b2f2eaf7809ae97b` |
| `trip.pl` | `93b38cc794f0c4a462667e25ef34a83552cbcdd62a42b10f739a431166525a79` |
| `trip.tfm` | `2c94bdba9c769e885f357823a183aaa5d2267731075f040f2a03cf6442a26181` |
| `tripin.log` | `ba01328756a8901d7c38162c9012014e9540322bf0963e105286f2a6ccb494cc` |
| `trip.log` | `61a653523bdccab9fd3f9aa61d170d0198c322c951938327b7daef9b70f26d8b` |
| `trip.fot` | `89e275ac12d025c06022e8dd6eb556b765954af2654b39ac2fbd451cf631b370` |
| `trip.typ` | `64efc62b962c592c2973f8c45a78e9e5d473f8b9da53ee53bc275a98041675cc` |
| `trip.dvi` | `09802695e330d34acec9192c15debe2de65e34fcbd3f947db9c8924240b1fe0a` |
| `tripos.tex` | `ea7447c7a8f2de278d2f84474f22c48c9d8a0059d7e16edd578d0bbe7077b47f` |
| `tripman.tex` | `a3e47254ad87fc3fdba210d61764c93b021740f56465971f5a41103405add48b` |

The exact URLs live in `tests/trip-manifest.txt` beside the matching hashes.

## Required Tools

The font phase requires `pltotf` and `tftopl`, overridable with
`UMBER_REF_PLTOTF` and `UMBER_REF_TFTOPL`. The DVItype phase requires
`dvitype`, overridable with `UMBER_REF_DVITYPE`.

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

## Normalization

The harness applies only these normalizations:

- `trip.log` and `tripin.log`: first-line date/time suffix and local
  `./trip.tex` path spelling.
- `trip.fot`: local `./trip.tex` path spelling.
- `trip.typ`: DVItype packaging text on line 1 and the rendered DVI preamble
  timestamp line.
- `trip.dvi`: DVI preamble comment bytes only, preserving the original comment
  length.

No other TRIP log, terminal photo, DVItype, `tripos.tex`, or DVI bytes may
change. Any mismatch writes a diff or byte context under `target/trip/diffs/`.

## Current Divergence Policy

The harness is allowed to expose current Umber failures; it must not normalize
or bless them. Missing INITEX/format support or semantic TRIP failures should
be filed as linked Beads work under `umber2-sfc` rather than hidden in this
script.
