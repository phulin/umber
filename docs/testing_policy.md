# Rust Testing Policy

Status: repository policy
Scope: forward-looking guidance for how agents should design, place, and run
Rust tests and fixtures in this workspace.

For the current test commands, corpus layout, harnesses, and measured budgets,
see [Testing Infrastructure](testing_infrastructure.md).

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

## 2. Test Tiers

The correctness tier is fixture-only and hermetic:

```bash
cargo test --tests
cargo test --workspace --tests
scripts/check-and-test.sh
```

These commands must not require `pdftex`, `tex`, `tftopl`, or other TeX tools
on `PATH`. Keep the default correctness tier fast enough to run routinely.
Move expensive scaling and live-reference checks into explicit performance or
regeneration tiers instead of weakening coverage in the default tier.

Regenerate committed fixtures only through `scripts/regen-fixtures.sh`, the
blessed live-reference rewrite path.

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

## 7. Fixture Policy

Committed corpus fixtures belong under the workspace-level `tests/corpus`
tree. Keep small area-local support files beside the fixture input. Test code
should live near the crate that owns the behavior; fixture data should live in
the shared corpus tree unless it is strictly local to one crate-level
integration test.

Default cargo tests must consume committed fixtures without invoking live TeX
tools. Licensing-sensitive external-document tests may conditionally consume
gitignored local oracles. Regenerate fixtures and local oracles only through
`scripts/regen-fixtures.sh`; setup scripts may orchestrate that path but must
not implement an independent generator or cargo-test environment switch.

Reference-derived fixtures must record enough provenance to reproduce and
audit them. Preserve byte-identical comparison except for explicitly
documented normalization. External inputs must be content-pinned and remain
uncommitted unless their redistribution policy explicitly permits committing
them.

See `tests/AGENTS.md` for fixture layout and regeneration instructions, and
[Testing Infrastructure](testing_infrastructure.md) for the current corpora
and harness inventory.

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
