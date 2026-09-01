# Rust Testing Policy

Status: repository policy
Scope: forward-looking guidance for how agents should design, place, and run
Rust tests and fixtures in this workspace.

For the current test commands, corpus layout, harnesses, and measured budgets,
see [Testing Infrastructure](testing_infrastructure.md).

---

## 1. Goals

Test placement should optimize for three things:

1. **Fast local gates.** `cargo test --tests` should remain fast
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
scripts/check-and-test.sh
```

The combined gate builds the complete native suite first, then executes the
prebuilt tests under a 6 GiB process-group guard while the quality gate runs in
parallel. Keeping the two cold Cargo builds sequential prevents test and
clippy compilation from competing for CPU and memory in a fresh worktree.

These commands must not require `pdftex`, `tex`, `tftopl`, or other TeX tools
on `PATH`. Keep the default correctness tier fast enough to run routinely.

The one prerequisite the correctness tier does impose is the four locally
generated, gitignored `tests/corpus/e2e/*.expected.dvi` oracles. Those gates
compare bytes against a real reference engine and are deliberately allowed to
fail rather than skip when their oracle is absent, because a skipped byte-exact
parity gate that reports success is worse than a red one. Materialize them once
in the primary checkout with `python3 scripts/provision.py worktree .`, then run
`python3 scripts/provision.py worktree <worktree>` while allocating each linked
worktree. Tests consume those assets but never provision them. An environment
that genuinely cannot host them opts out explicitly with
`UMBER_CONFORMANCE_ORACLES=optional`. See the End-to-End Conformance Gate Contract in
[Testing Infrastructure](testing_infrastructure.md).
Move expensive scaling and live-reference checks into explicit performance or
regeneration tiers instead of weakening coverage in the default tier.

`cargo test --tests` selects the workspace's `default-members`, and that list
now names every host-testable member, so the plain command is the whole tier.
It once named 21 of 34, which silently left the nine `bib-*` crates,
`umber-interrupt`, `refexec`, and `profile-analyzer` executed by no routine
command at all (`umber2-johp.211`).

The fix is a test rather than a wrapper. `default_members_cover_every_host_testable_crate`
in `test-support` reads `cargo metadata` and fails if any member is absent from
`default-members` without a declared reason naming the check that runs it, and
a companion test does the same for every `[workspace] exclude` directory, which
`--workspace` cannot reach at all. The coverage invariant is therefore enforced
inside the suite, under the command everyone already runs -- strictly better
than being enforced by remembering to invoke a particular script.

`cargo test -q --tests -p <crate>` remains the right command while iterating on
one crate.

The routine `workspace_selection` test executable also audits active source
authority. Production Rust may not use the unconditionally false
`#[cfg(any())]`, and a library that disables its unit-test target with
`[lib] test = false` may not colocate `#[cfg(test)]` modules. The exception set
is empty: new sites fail, and the scanner's positive tests prove that any future
reviewed exception must also fail when it becomes stale. Deliberate positive
and negative source fixtures keep both rejection paths and their remediation
diagnostics executable.

Only a test selected and executed by a named routine or explicit gate is active
evidence. A dormant source file, a disabled test target, a dead conditional, or
a reviewed migration exception is inventory, not coverage; documentation and
property catalogues must cite an active test instead.

`umber-wasm` is the one declared exclusion. Its tests are
`#[wasm_bindgen_test]`, which registers no test on a host target, so selecting
it would build a cdylib and run exactly zero tests; `scripts/check-wasm.sh`
runs them for real under `wasm-pack test --headless --firefox`. Host-side
regeneration, profiling, and triage entry points run through
`scripts/check-tools.sh`, which also covers `parity-harness` in its
`reference-tools` resolution.
Its `profiling-command-tests` step compiles `tex-command`'s complete unit-test
target with the `profiling` feature enabled and runs the focused resident-input
fixture. Building a profiling-enabled Umber target alone compiles only the
dependency library, not `tex-command`'s feature-only test bodies.

These checks stay outside the routine tier so it does not depend on wasm-pack,
a browser, ripgrep, HarfBuzz, pinned distributions, or additional dependency
trees. Run them explicitly when working in the subsystem they cover. Their
shared runner reports missing prerequisites as `BLOCKED`, not success.

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

TeX test inputs must state whether they are complete jobs or host-owned
fragments. A complete job includes the terminator appropriate to the surface
under test: normally `\end`, `\dump` for an INITEX format build, `\bye` when
Plain's cleanup is part of the behavior, or `\end{document}` for LaTeX. Include
files, scanner snippets, generated property-test chunks, and editor buffers may
instead use the explicit fragment harness, which stops at root EOF without
pretending that TeX scanned `\end` or running final cleanup. Harnesses must not
silently append a terminator: doing so can hide scanner state, grouping, page
output, and final-cleanup defects. A test of missing-terminator behavior is a
complete job by definition and must assert the mode-specific diagnostic and a
bounded terminal result.

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

Cargo features are governed separately by
[Cargo Feature Axes](cargo_feature_axes.md), which decides what a feature may
mean and which crate declares it; a test that needs a new build configuration
should read that contract before adding one.

See `tests/AGENTS.md` for fixture layout and regeneration instructions, and
[Testing Infrastructure](testing_infrastructure.md) (reference doc only, read
only if necessary) for the current corpora and harness inventory.

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
