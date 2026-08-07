# Detached PDF finalization closeout

Issue: `umber2-vgjr.6`

Implementation tree: `710ca9de0a66f7e513d1a93289feb5ad9b76b61a`.

## Surviving authorities

`tex-out::pdf::PdfFinalizationInput` is the sole complete host-neutral PDF
handoff. `tex_out::pdf::finalize_pdf` owns form validation, artifact and page
lowering, recursive VF execution, font usage and font-object emission, raster
and selected-page resource import, navigation and object construction,
allocation validation, graph validation, and deterministic serialization
through `pdf-writer`. Repository-wide searches found one production call and
no alternate serializer, legacy finalizer, runtime fallback, or duplicate
selected-page resource importer.

Umber is the host/session adapter. It expands token lists, reads committed
artifacts and raw-object inputs, freezes validated resource values, translates
errors and diagnostics, and replays only the allocation receipt already proven
against a private cloned document ledger. Its retained PDF parser inspects a
requested source page's metadata before detachment; it neither imports that
page's resource graph nor emits output. A failed detached build publishes no
bytes and does not advance the live document ledger.

`tex-fonts` remains the validation authority for TFM, VF, PK, Type 1, and
TrueType data and canonical realized identities. Each detached VF local-font
record retains exact `Arc`-backed TFM bytes, their VirtualFile-domain content
identity, and the design-size validation receipt. `tex-out` recomputes the
identity, reparses the receipt and every packet-declared size through
`tex-fonts`, and rejects byte, hash, identity, cycle, depth, work, stack,
special-byte, output, and arithmetic violations without a host callback.

## Independent evidence

The committed PDF corpus supplies exact Umber bytes, independently normalized
Hayro structure, pinned pdfTeX structure, and digest-bound Poppler render and
text attestations. Focused graph, form, font, image, navigation, allocation,
resource-limit, serialization, and external-import tests remain active. The
detached-only nested-VF case proves the 12pt -> 6pt -> 9pt size chain, exact
leaf width and TFM transport, resource/object order, tamper rejection, cycle
and depth rejection, successful live replay, and independent Hayro parsing.

The local external gate precisely reports the pinned qpdf and Poppler tools as
unavailable in this environment. A version-correct validator stand-in then
exercised the repaired native CLI artifact path: all three object-compression
levels plus raster PNG, alpha PNG, and DCT JPEG were generated, found, and
presented to the complete qpdf matrix. This is artifact-generator evidence,
not a claim that qpdf or Poppler ran.

## Exact accounting

Commit arithmetic across `6618b3441`, `df84c8c88`, `d0b62af5a`, `d96b1742a`,
`c6a9d8b91`, `dd8802031`, and `710ca9de0` is:

| Category                     | Additions |  Deletions |        Net |
| ---------------------------- | --------: | ---------: | ---------: |
| Production Rust              |     6,412 |     10,091 |     -3,679 |
| Active proof Rust            |       213 |        102 |       +111 |
| External-validator script    |        24 |          8 |        +16 |
| Manifests and lockfile       |         6 |          0 |         +6 |
| Documentation and guidance   |        68 |         26 |        +42 |
| **Complete tracked program** | **6,723** | **10,227** | **-3,504** |

The original 800--1,400-line production forecast is retired in favor of the
measured 3,679-line production reduction. Generated source, PDF fixtures,
binary assets, and new dependency packages are zero; no moved or future
compatibility-gated code is credited.

## Verification

With `CARGO_BUILD_JOBS=6`, the complete native suite compiled uncapped.
Runtime gates used finite timeouts, `MemoryMax=512M` for focused work and
`MemoryMax=1G` for the complete suite, and never used `prlimit`. Under 512 MiB,
all 79 `tex-fonts`, 151 `tex-out`, 40 active `tex-incr`, 369 active Umber unit,
three Umber boundary, and 116 active Umber integration tests passed. This
includes the PDF, VF, maximum-depth DVI and positioned geometry, incremental,
corpus, committed byte/structure/render-attestation parity, resource, and
failure-atomic publication gates. The complete `cargo test -q --tests` suite
passed under 1 GiB. Final uncapped `scripts/check.sh` passed all four gates.
