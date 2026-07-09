# Rust Testing Policy

Status: repository policy
Scope: where Rust test code and fixtures should live in this workspace.

---

## 1. Goals

Test placement should optimize for three things:

1. **Fast local gates.** `cargo test --workspace --tests` should remain fast
   enough to run often. Long parity runs belong in focused scripts, not in the
   default cargo test path.
2. **Clear production files.** Source files should stay short and focused so
   humans and agents can read implementation code without paging through large
   test tables, fixtures, or helper scaffolding.
3. **Correct Rust boundaries.** Tests should live at the visibility boundary
   they are actually validating: internal-library tests under `src`, external
   boundary tests under crate-level `tests`, and shared fixture data under the
   workspace `tests/corpus` tree.

## 2. Test Tiers And Budgets

The default tier is fixture-only and hermetic:

```bash
cargo test --tests
cargo test --workspace --tests
scripts/check.sh
```

These commands must not require `pdftex`, `tex`, `tftopl`, or other TeX tools
on `PATH`. `scripts/check.sh` forces `UMBER_LIVE_REF=0` and
`UPDATE_FIXTURES=0` so it remains the local fast gate even when a developer's
shell has reference-mode variables set. The warmed `cargo test --tests` target
is under 10 seconds on the current macOS development workspace; investigate a
sustained run above 15 seconds or any default test that invokes live TeX. The
broader `scripts/check.sh` gate also runs format and clippy, including release
clippy, and should stay under a warmed two-minute local budget.

The explicit slow parity tier is:

```bash
scripts/parity.sh
```

That script enables `UMBER_LIVE_REF=1`, runs live reference diagnostic checks,
optional hyphenation parity when its corpus is present, and byte-identical DVI
comparisons for the DVI, page, math, align, and leaders corpora. Use focused
`UPDATE_FIXTURES=1` commands only when deliberately regenerating committed
fixtures.

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
`tests/AGENTS.md` for fixture layout and update commands.

The DVI corpora under `tests/corpus/dvi`, `tests/corpus/page`,
`tests/corpus/math`, `tests/corpus/align`, and `tests/corpus/leaders` commit
TeX source files plus `.expected.dvi` reference fixtures. The default `umber`
cargo tests run every `.tex` case in those areas against the committed DVI
fixtures and do not invoke live reference tools. The explicit slow tier remains
in `scripts/parity.sh`: it runs `umber run --dvi` and the live reference engine
over the same temporary inputs with pinned local TFMs, then compares generated
DVI bytes through `tools/refexec`. The math corpus uses a shared primitive-only
`tests/corpus/math/math_preamble.inc` include and the parity script runs its
reference side in INITEX mode so the cases do not depend on `plain.tex`.

Default cargo tests must not invoke live TeX tools. Fixture regeneration uses
`UPDATE_FIXTURES=1` and focused tests, while live reference checks use
`UMBER_LIVE_REF=1` or `scripts/parity.sh`.

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
