# test-support Guidance

Read the repository-level `AGENTS.md` before editing here. This crate contains host-side utilities for tests and fixture comparison; it is not part of the TeX engine runtime.

`git_fixture` validates small repository-owned cases against the selected
runtime Git checkout. Its `case.inventory` schema closes both the tracked and
on-disk file sets. Every directory component from that checkout through the
case root is inspected without following links, so alternate-checkout,
`target`/generated/scratch, symlink, and non-regular authority are forbidden
before canonical traversal.
Unmanifested minifixtures use the same validator with Git itself as the exact
inventory. Family inventory tests own only enumeration and census.
Metadata roles resolve through
`ClosedCase::payload_path`, which accepts only declared single-filename
payloads and revalidates the no-follow ancestry, selected-checkout identity,
tracked/declared/filesystem inventories, and selected entry's regular-file
type before every access.
`closed_case::FixtureCase` is the common consumer boundary for both
`case.inventory` and Git-inventoried families. It pairs that authority with
typed identity, ordered roles, source closure, profile, status, and publication
metadata; classic BibTeX additionally imports its declared roles and hashes.
Its candidate staging preserves each family's manifested or unmanifested
shape, and its canonical inventory serializer is shared by fixturegen.

## Crate Role

`test-support` owns shared helpers used by workspace tests, especially committed corpus fixture assertions, normalized diagnostic/log comparison, DVI fixture setup/comparison, and small parsers used by regeneration tooling to cross-check reference tool output. It may depend on ordinary host libraries such as `anyhow` and diffing utilities because it runs only in tests and host tools.

Keep reusable test harness code here when multiple crates or integration tests need the same fixture, normalization, or reference-comparison behavior. Keep crate-specific assertions near the crate that owns the behavior unless they are clearly shared.

## File Map

- `AGENTS.md`: crate-specific guidance, boundaries, validation notes, and this file map.
- `Cargo.toml`: crate manifest, host-side fixture dependencies, and workspace lint settings.
- `src/closed_case.rs`: typed closed-case identity, conventional and classic-BibTeX consumer adapters, ordered file roles, status/xfail, profile, source-closure and publication metadata validation, canonical inventory serialization, plus non-authoritative local candidate staging.
- `src/closed_case/tests.rs`: contract, hash, order, closure, traversal, local-edit, and staging compatibility coverage.
- `tests/tex82_catalogue.rs`: hermetic TeX82 property-catalogue completeness, citation, ownership, and exact test-link gate.
- `tests/workspace_selection.rs`: routine workspace-selection and release-surface audits.
- `tests/workspace_selection/source_audit.rs`: routine inactive-test-authority source audit, exact reviewed migration exceptions, and positive/negative audit fixtures.
- `src/compile_fail.rs`: Shared offline Cargo-check harness that gives each compile-fail fixture an independent temporary crate, points every crate at one reusable target directory, detaches nested Cargo from the outer test jobserver, and checks stable stderr substrings.
- `src/corpus.rs`: shared committed-corpus discovery and support-file copy helpers.
- `src/dvi.rs`: shared DVI fixture setup, preamble-comment normalization, exact comparison, and byte-difference context.
- `src/lib.rs`: public fixture assertion/read helpers, checked runtime-checkout asset reads, TeX/reference log normalizers that retain canonical error context (including indented continuation lines) while dropping the explicitly delimited PDF-statistics block, and PL font parsing utilities.
- `src/git_fixture.rs`: selected-checkout Git and filesystem authority validation for closed cases, preserving manifest order and revalidating before payload access.
- `src/bin/pdf-normalize.rs`: host-only CLI exposing the independent Hayro
  structure projection to live-reference tooling.
- `src/pdf.rs`: canonical PDF structure normalizer that walks shallow Hayro-backed values directly.
- `src/pdf/tests.rs`: inherited-resource merging and stable cycle-notation normalization coverage.
- `src/pdf_query.rs`: bounded Hayro document with shallow borrowed object,
  dictionary, array, and page handles plus focused owned stream-byte and
  content-operation queries. This is the semantic assertion boundary for PDF
  consumers and must not grow a recursive owned object graph.
- `src/pdf_query/tests.rs`: classic/xref-stream, object-stream, budget, cycle, inheritance, raw/decoded-stream, operation-order, and malformed-input query coverage.
- `src/pdf_query/fixtures/xref-object-stream.pdf`: committed uncompressed xref-stream/object-stream compatibility fixture.
- `src/pdf_fixture.rs`: tiny `pdf-writer` adapter for ordinary valid synthetic
  inputs plus an explicitly separate handcrafted classic-xref builder for
  malformed, cyclic, depth-limit, and writer-independent evidence.
- `src/tests.rs`: crate self-test that reads the committed hello fixture.

## Boundaries

- Do not make engine crates depend on `test-support` outside dev-dependencies.
- Do not put production TeX logic here; helpers in this crate may normalize, compare, or parse expected data, but they should not become an alternate implementation of runtime behavior.
- Keep reference-tool assumptions explicit and isolated in `scripts/regen-fixtures.sh`
  and tooling, not in cargo tests.

## Validation

Changes here usually need the tests that consume the helper plus `cargo test --tests -p test-support`. When fixture output changes, follow `tests/AGENTS.md` and regenerate with `scripts/regen-fixtures.sh`.
