# Rust Testing Policy

Status: repository policy
Scope: where Rust test code and fixtures should live in this workspace.

---

## 1. Goals

Test placement should optimize for three things:

1. **Fast local gates.** `cargo test --workspace --tests` should remain fast
   enough to run often and is the correctness gate against committed fixtures.
   Live reference work belongs in fixture regeneration, not in cargo tests.
2. **Clear production files.** Source files should stay short and focused so
   humans and agents can read implementation code without paging through large
   test tables, fixtures, or helper scaffolding.
3. **Correct Rust boundaries.** Tests should live at the visibility boundary
   they are actually validating: internal-library tests under `src`, external
   boundary tests under crate-level `tests`, and shared fixture data under the
   workspace `tests/corpus` tree.

## 2. Test Tiers And Budgets

The correctness tier is fixture-only and hermetic:

```bash
cargo test --tests
cargo test --workspace --tests
scripts/check-and-test.sh
```

These commands must not require `pdftex`, `tex`, `tftopl`, or other TeX tools
on `PATH`. The warmed `cargo test --tests` target is under 10 seconds on the
current macOS development workspace; investigate a sustained run above 15
seconds or any default test that invokes live TeX. The broader
`scripts/check.sh` quality gate runs format and clippy without rerunning tests
and should stay under a warmed two-minute local budget. Use
`scripts/check-and-test.sh` for the full workspace test suite followed by that
quality gate.

Regenerate committed fixtures only through `scripts/regen-fixtures.sh`, which
is the blessed live-reference rewrite path. Its `--area fonts` mode owns the
explicit live `tftopl` cross-check and does not rewrite fixtures.

## 3. Default Rule

Put nontrivial crate-internal tests in a separate sibling test module:

```rust
// src/foo.rs
#[cfg(test)]
mod tests;
```

with test code in:

```text
src/foo/tests.rs
```

For `src/lib.rs`, use:

```text
src/tests.rs
```

or, when the suite is large:

```text
src/tests.rs
src/tests/<topic>.rs
src/tests/support.rs
```

This keeps implementation files compact while preserving unit-test access to
private and `pub(crate)` implementation details.

## 4. Inline Tests

Inline `#[cfg(test)] mod tests { ... }` blocks are allowed only when the test
block is small and genuinely local to the implementation, roughly 20 to 40
lines.

Good uses:

- arithmetic edge cases
- constructor invariants
- tiny parser/scanner examples
- one or two regression tests tied directly to a private helper

Move tests into `tests.rs` once they need setup helpers, table-driven cases,
fixtures, many assertions, or more than a few test functions.

## 5. Crate-Level Integration Tests

Internal library crates should avoid crate-level Cargo integration tests.
Prefer `src/tests.rs` and `src/tests/<topic>.rs` even when a test exercises
many modules together; those still compile as one crate unit-test binary and
can use internal APIs without widening production visibility.

Use `crates/<crate>/tests/` only for tests that intentionally exercise an
external boundary. These tests should normally use only public APIs and should
be reserved for:

- capability and visibility boundaries
- CLI behavior
- cross-crate behavior
- replay identity
- fixture and parity tests
- compile-fail UI tests

Avoid using crate-level integration tests for white-box implementation details
or internal-library regression suites. If a test needs private access, or if it
is validating an internal crate's implementation rather than an external
contract, it belongs under `src`.

## 6. Large Integration Suites

Cargo compiles each top-level file under `tests/` as a separate test crate.
Any crate that keeps integration tests should have at most one top-level Cargo
integration test binary unless there is a measured reason to split it. Prefer
one test binary with submodules:

```text
crates/foo/tests/it.rs
crates/foo/tests/it/
  parity.rs
  cases.rs
  support.rs
```

This improves compile time, simplifies shared helpers, and keeps test output
easier to scan.

## 7. Fixtures

Committed corpus fixtures belong under the workspace-level `tests/corpus`
tree. Keep small area-local support files beside the fixture input. See
`tests/AGENTS.md` for fixture layout and the `scripts/regen-fixtures.sh`
regeneration modes.

The DVI corpora under `tests/corpus/dvi`, `tests/corpus/page`,
`tests/corpus/math`, `tests/corpus/align`, and `tests/corpus/leaders` commit
TeX source files plus `.expected.dvi` reference fixtures. The default `umber`
cargo tests run every `.tex` case in those areas against the committed DVI
fixtures and do not invoke live reference tools. `scripts/regen-fixtures.sh`
owns DVI fixture regeneration: it runs the live reference engine through
`tools/refexec`, copies the pinned local CM TFMs plus area support files, uses
INITEX for the math corpus, and rewrites raw reference DVI only when the
existing preamble-comment-only DVI comparison detects a change.

Default cargo tests must not invoke live TeX tools. Fixture regeneration uses
`scripts/regen-fixtures.sh`, which also builds `tools/fixturegen` for
text/native fixture updates and the live `tftopl` font cross-check.

External document corpus inputs for long-running parity live outside committed
fixtures. The line-oriented `tests/corpus-manifest.txt` file pins support files
and documents by URL, fetched-byte SHA-256, license determination, and
redistributability flag. Runnable documents also select a format source and pin
the reference DVI SHA-256 after DVI preamble banner normalization. `scripts/parity.sh` runs
`tools/corpus-sync` first to fetch or verify those inputs under gitignored
`third_party/corpus/`; cached hash matches are a no-op, including in
`--offline` mode. It also pins `SOURCE_DATE_EPOCH=1783604160` and
`FORCE_SOURCE_DATE=1` by default before running reference TeX so
date-sensitive documents have stable DVI body bytes. Do not commit fetched
corpus documents unless a later issue explicitly changes the redistribution
policy.

Full external-document DVI parity is an explicit script tier, not a cargo-test
tier:

```bash
scripts/parity.sh e2e
scripts/parity.sh e2e --offline
```

This mode verifies acquisition, runs reference TeX through `tools/refexec`,
checks the manifest-pinned normalized reference DVI hash for environment
drift under the script-pinned job clock, and byte-compares the normalized DVI
files. Each document names a manifest-pinned `format_source`; the harness
stages that exact source, the document, hyphenation input, and required TFMs,
then feeds both engines the same wrapper that inputs the format source before
the document. Reference TeX uses INITEX mode and Umber executes the wrapper
through its ordinary input path. This follows TeX82's ordinary
`start_input` stack behavior (sections 23 and 29); format dumping remains a
terminal INITEX cleanup operation (sections 46, 50, and 51), not a way to
continue into the document. The pinned modern `plain.tex` source contains no
`\\dump`, so the unmodified file can be loaded directly. On reference drift,
Umber failure, or mismatch it writes a
triage bundle under
`target/parity-triage/<doc-name>/` with byte context, page-limited
dvitype-style disassemblies, a unified disassembly diff, tracing-output logs,
and a summary that names the divergent page and opcode when a page can be
recovered from DVI backpointers. `scripts/parity.sh self-test` exercises the
bundle writer with synthetic DVI and remains fast enough for local tooling
checks, but the external corpus itself must stay outside
`cargo test --workspace --tests`.

The original Knuth TeX82 TRIP test is a separate explicit conformance tier,
not part of the default Rust test tier and not part of the later e-TRIP work:

```bash
scripts/trip.sh
scripts/trip.sh --offline
scripts/trip.sh self-test
scripts/build-trip-initex.sh
```

`scripts/trip.sh` reads `tests/trip-manifest.txt`, fetches exact official
TRIP bytes into gitignored `third_party/trip/`, and verifies every SHA-256
before running. It rebuilds `trip.tfm` from `trip.pl` with PLtoTF, converts it
back with TFtoPL, compares the generated TFM to the canonical CTAN TFM, then
runs the documented INITEX and format-loaded TRIP phases. This tier requires
Knuth's special TRIP INITEX build described in `tripman.tex` Appendix A; stock
`pdftex -ini` or `tex -ini` is useful only as a failing sanity check because
the official log line widths, memory limits, and capacity statistics depend on
that special build. Set `UMBER_TRIP_INITEX=/absolute/path/to/initex` to select
it. `scripts/build-trip-initex.sh` builds the hash-pinned TeX Live Web2C
classic TeX plus DVItype, PLtoTF, and TFtoPL tools; once its source archive is
cached, both the build and reference phase run offline. The harness
automatically uses `target/trip-initex/bin`, or `UMBER_TRIP_TOOLS` can select
another pinned build. It also uses `UMBER_REF_PLTOTF`, `UMBER_REF_TFTOPL`, and
`UMBER_REF_DVITYPE` overrides when the TeXware tools are not on `PATH`.

Allowed TRIP normalization is executable and narrowly bounded: environment
banner/date/path and Appendix-A capacity statistics, box `glue set` deltas no
larger than 0.001, and DVI movement deltas no larger than 64 scaled points with
identical structure. Characters, rules, specials, box dimensions, page/font
structure, non-movement DVI operands, `tripos.tex`, and all other text must
match exactly. Failures write
unified diffs or byte contexts under `target/trip/diffs/`; `scripts/trip.sh
self-test` deliberately perturbs a copied text artifact and verifies that this
diff path is actionable without fetching or running TeX.
See [trip.md](trip.md) for the exact source pins and normalization policy.

`tex-out` also owns the cross-crate page-output float guard. Its unit tests
scan the page node, packing, shipout lowering, artifact, DVI, and CLI DVI
composition sources and fail if float types or float rounding APIs enter that
fixed-point path. Keep the guard allowlist limited to documented
non-arithmetic fixture or formatting false positives.

Test code should live near the crate that owns the behavior. Fixture data
should live in the shared corpus tree unless it is strictly local to one
crate-level integration test.

## 8. Documentation Tests

Use doctests only when the example is part of public API documentation and is
valuable to users as documentation. Do not use doctests as the main test
mechanism for internal crates or implementation behavior.

For internal crates with many examples, prefer normal Rust tests so compile
time and test organization stay predictable.

## 9. Navigation Rules For Agents

When adding or moving tests:

- Keep production modules readable without requiring test context.
- Prefer `#[cfg(test)] mod tests;` over large inline test blocks.
- Mirror the implementation path where practical: `src/foo.rs` gets
  `src/foo/tests.rs`; `src/foo/mod.rs` gets `src/foo/tests.rs` or
  `src/foo/tests/<topic>.rs`.
- Use `support.rs` only for helpers shared by nearby tests.
- Keep helper APIs test-only unless they are part of the production design.
- Do not expose production internals just to make a test fit in
  crate-level `tests/`.

## 10. External References

Rust's documented convention is that unit tests live under `src` and can test
private interfaces, while integration tests live under top-level `tests/` and
exercise the crate like external code:

- [The Rust Book: Test Organization](https://doc.rust-lang.org/book/ch11-03-test-organization.html)

The separate-file unit-test style is common Rust practice and keeps large
source files readable:

- [Rust forum: Should unit tests really be put in the same file as the source?](https://users.rust-lang.org/t/should-unit-tests-really-be-put-in-the-same-file-as-the-source/62153)

For large projects, Cargo's integration-test compilation model matters because
each top-level integration test file becomes a separate test binary:

- [matklad: Delete Cargo Integration Tests](https://matklad.github.io/2021/02/27/delete-cargo-integration-tests.html)
- [matklad: Unit and Integration Tests](https://matklad.github.io/2022/07/04/unit-and-integration-tests.html)
