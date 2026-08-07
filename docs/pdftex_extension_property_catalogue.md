# pdfTeX Extension Property Catalogue

Status: implemented retained-fixture ownership contract.

The pdfTeX extension catalogue is separate from the TeX82 module catalogue.
Its sole canonical source is pdfTeX 1.40.29's `pdftex.web`, pinned at TeX Live
source commit `1664cf0ab3f6ce3b80db649bc6723f54ab12016c` with SHA-256
`5a105669acc1b49aedb7560d4d15cb2e23467cb16d895eb0031c8dd9fea32f04`.
Numeric `sections` in the catalogue are numbered WEB sections in that source.

The catalogue does not enumerate primitives. The exact 158-name primitive
inventory remains owned by `docs/pdftex_primitives.md` and its source-derived
test. This avoids a second table that could silently become a shadow authority.

## Contract

`tests/pdftex-properties/catalogue.json` assigns every retained
`tests/corpus/tex_exec/pdf_*` case exactly one stable `pdftex.extension.*`
property ID. Each property has:

- pinned `pdftex.web` sections and a paraphrased claim;
- one semantic owner and one exact active `#[test]` link;
- the oracle's success status; and
- explicit status, terminal, and log projections.

Each observation is either `pass` or a strict `xfail` naming a Beads bug. The
active runner requires a passing projection to remain present and an xfail
projection to remain absent. An implementation fix therefore makes the xfail
test fail until the bug is closed and the observation is deliberately promoted
to `pass`; a known mismatch cannot be absorbed as success.

`crates/test-support/tests/pdftex_extension_catalogue.rs` is the hermetic
completeness gate. It rejects source-pin drift, duplicate property IDs,
overlapping or incomplete case ownership, invalid section citations, missing
oracle channels, projections absent from the preserved reference, unlinked
xfails, and dormant or ambiguous Rust test links. The gate derives its case
inventory from the closed retained corpus instead of pinning only a count.

The eight cases previously blocked under `umber2-alfh.29` are actively executed
by `retained_pdftex_extension_fixtures_compare_oracle_projections`. It stages
typed generated PNG, JPEG, and three-page PDF inputs for the image enquiry case
and compares all three channels for every case. Known navigation-finalization,
fatal-diagnostic, image-allocation, and recovered-error status differences
remain visible under `umber2-alfh.33`, `umber2-alfh.34`, `umber2-alfh.35`, and
`umber2-alfh.36`; they do not weaken or replace the unique historical
references.

## Validation and regeneration

Routine validation is fixture-only:

```bash
cargo test -q -p test-support --test pdftex_extension_catalogue
cargo test -q -p umber --lib retained_pdftex_extension_fixtures_compare_oracle_projections
```

`scripts/regen-fixtures.sh --area tex_exec` remains validation-only because the
historical references predate a reproducible capture pin. It runs the active
consumers and never rewrites `expected.ref`; live oracle work uses the pinned
pdfTeX regeneration workflow instead.
