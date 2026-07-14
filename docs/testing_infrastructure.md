# Testing Infrastructure

Status: current repository reference
Scope: the test commands, measured budgets, fixtures, corpora, and harnesses
that exist in this workspace today.

This document records current implementation facts. For rules that should
guide future test design and placement, see [Rust Testing Policy](testing_policy.md).

---

## Local Gates And Budgets

The fixture-only, hermetic correctness tier is:

```bash
cargo test --tests
cargo test --workspace --tests
scripts/check-and-test.sh
```

The warmed `cargo test --tests` target is under 10 seconds on the current
macOS development workspace; investigate a sustained run above 15 seconds or
any default test that invokes live TeX. `scripts/check.sh` runs format and
clippy without rerunning tests and has a warmed two-minute local budget.
`scripts/check-and-test.sh` runs the full workspace test suite followed by that
quality gate.

Snapshot scaling has a separate explicit performance tier:

```bash
cargo bench --manifest-path benchmarks/tex-state/Cargo.toml --bench snapshot_budgets
scripts/check-snapshot-budgets.sh
```

The Criterion command records the small/large latency rows. The script enforces
the low-noise latency ratio and requested-allocation retention budgets described
in [Snapshot Performance](snapshot_performance.md). Neither belongs in the
default cargo-test tier because its workload deliberately materializes large
input, page, mode, stream, hyphenation, provenance, and Unicode code-table
state.

Whole-engine Gentle profiling has a separate persistent in-process runner:

```bash
scripts/profile-gentle.sh
```

It preloads external corpus and font inputs into a structurally shared memory
World, performs a warm-up, then repeats fresh engine sessions without
per-iteration temporary-directory or host-file staging. The script builds an
optimized symbolized binary and saves the Samply profile under
`target/profiles/`. See [Profiling Umber with Gentle](profiling.md) for its
controls and measured boundary.

## Fixture Regeneration

`scripts/regen-fixtures.sh` is the sole live-reference rewrite path. It builds
`tools/fixturegen` for text/native fixture updates and `tools/refexec` for DVI
fixture updates. Its `--area fonts` mode owns the explicit live `tftopl`
cross-check and does not rewrite fixtures.

See `tests/AGENTS.md` for the supported areas and cases, required tools,
copied support files, and validation performed after a rewrite.

## Committed DVI Corpora

The DVI corpora under `tests/corpus/dvi`, `tests/corpus/page`,
`tests/corpus/math`, `tests/corpus/align`, and `tests/corpus/leaders` commit TeX
source files plus `.expected.dvi` reference fixtures. The default `umber` cargo
tests run every `.tex` case in those areas against the committed DVI fixtures
without invoking live reference tools.

DVI regeneration runs the live reference engine through `tools/refexec`,
copies the pinned local CM TFMs and area support files, uses INITEX for the math
corpus, and rewrites raw reference DVI only when the existing
preamble-comment-only comparison detects a change.

## External Document Corpus

External document inputs live outside committed fixtures. The line-oriented
`tests/corpus-manifest.txt` pins support files and documents by URL, fetched-byte
SHA-256, license determination, and redistributability flag. Runnable documents
also select a format source and pin the reference DVI SHA-256 after DVI preamble
banner normalization.

`scripts/setup-conformance-tests.sh` builds `tools/corpus-sync` to fetch or
verify those inputs under gitignored `third_party/corpus/`, then acquires the
remaining local support files and generates all four end-to-end DVI oracles.
Cached hash matches are a no-op. Fixture regeneration pins
`SOURCE_DATE_EPOCH=1783604160` and `FORCE_SOURCE_DATE=1` so date-sensitive
documents have stable DVI body bytes. Once setup completes, the conformance
tests consume only local files and require no network access.

Full external-document DVI parity is exposed as local-oracle-backed Cargo
integration tests:

```bash
cargo test -p umber --test it e2e_conformance_story -- --nocapture
cargo test -p umber --test it e2e_conformance_gentle -- --nocapture
```

Populate the external inputs and all Story, Gentle, TRIP, and e-TRIP DVI oracles with
`scripts/setup-conformance-tests.sh`. The generated `.expected.dvi` files are
gitignored licensing-sensitive derivatives and are not repository fixtures.

The shared `parity-harness` library stages inputs, calls the Cargo test's in-process Umber
runner, and byte-compares its normalized DVI with the local `tests/corpus/e2e`
oracle. Each document names a manifest-pinned
`format_source`; the harness stages that source, the document, hyphenation
input, and required TFMs, then feeds Umber a wrapper that inputs the format
source before the document through the ordinary input path.

This follows TeX82's ordinary `start_input` stack behavior (sections 23 and
29). Format dumping is a terminal INITEX cleanup operation (sections 46, 50,
and 51), not a way to continue into the document. The pinned modern
`plain.tex` contains no `\\dump`, so it can be loaded directly.

On fixture-hash drift, Umber failure, or mismatch, the harness writes a triage
bundle under `target/conformance-triage/<doc-name>/` with byte context,
page-limited dvitype-style disassemblies, a unified diff, tracing logs, and a
summary naming the divergent page and opcode when recoverable from DVI
backpointers. The `cargo test -p parity-harness self_test_bundle_pinpoints_page_and_opcode`
command exercises the bundle writer with synthetic DVI.
`scripts/regen-fixtures.sh --case e2e/story` and `--case
e2e/gentle` verify the manifest-pinned normalized reference hash before
rewriting either fixture.

## TRIP Corpus

The original Knuth TeX82 TRIP and e-TeX V2 e-TRIP workloads are end-to-end DVI
conformance tests that run conditionally when their local inputs and oracles
are present:

```bash
scripts/trip.sh
scripts/trip.sh --offline
scripts/trip.sh self-test
scripts/build-trip-initex.sh
cargo test -p umber --test it e2e_conformance_trip -- --nocapture
cargo test -p umber --test it e2e_conformance_etrip -- --nocapture
scripts/regen-fixtures.sh --case e2e/trip
scripts/regen-fixtures.sh --case e2e/etrip
```

`scripts/trip.sh` reads `tests/trip-manifest.txt`, fetches exact official TRIP
and e-TRIP bytes into gitignored `third_party/trip/`, and verifies every SHA-256 before
running. It uses the pinned canonical `trip.tfm`, then runs the documented
INITEX and format-loaded TRIP phases.

Cargo conformance tests do not invoke `scripts/trip.sh` or launch Umber as a
subprocess. Story and Gentle call the engine directly through the staged
fixture callback; TRIP and e-TRIP share one in-process two-phase format helper.

This tier requires Knuth's special TRIP INITEX build described in
`tripman.tex` Appendix A. Stock `pdftex -ini` or `tex -ini` is useful only as a
failing sanity check because the official log line widths, memory limits, and
capacity statistics depend on that special build.
`UMBER_TRIP_INITEX=/absolute/path/to/initex` selects it.
`scripts/build-trip-initex.sh` builds the hash-pinned TeX Live Web2C classic
TeX plus DVItype; after its source archive is cached, both the build and
reference phase run offline. The harness automatically uses
`target/trip-initex/bin`; `UMBER_TRIP_TOOLS` can select another pinned build,
and `UMBER_REF_DVITYPE` can select DVItype when it is not on `PATH`.

The Umber integration test gates only the final DVI. Generated logs, terminal
photo, and `tripos.tex` remain diagnostic outputs in the separate diagnostic
parity tier. Its oracle normalizes only the DVI preamble comment and otherwise
requires byte identity with the committed, locally pdfTeX-generated fixture.
Regeneration executes the two-phase workload from `trip.tex` and `trip.tfm`
and never copies the official `third_party/trip/trip.dvi`.

DVItype remains diagnostic. Standalone reference-engine validation retains
the narrowly bounded Appendix A movement reconciliation needed to validate the
special reference toolchain; that allowance is never applied to Umber's final
DVI. Failures write byte, page, opcode, and disassembly context under
`target/conformance-triage/trip/`. See [TRIP](trip.md) for the exact source
pins and normalization policy.

## Specialized Guards

`tex-out` owns the cross-crate page-output float guard. Its unit tests scan the
page node, packing, shipout lowering, artifact, DVI, and CLI DVI composition
sources and fail if float types or float rounding APIs enter that fixed-point
path. Its allowlist is limited to documented non-arithmetic fixture or
formatting false positives.
