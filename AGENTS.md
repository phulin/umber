# Repository Guidance

This project is a faster, more modern, and more portable reimplementation of
TeX, LaTeX, and pdfTeX in Rust.

Read this file first, then the nearest nested `AGENTS.md` before editing within
a subdirectory. Keep every `AGENTS.md` current when source files or
subdirectories are added, removed, or repurposed.

The project uses Beads (`bd`) for issue tracking and durable project memory.

## General Instructions

- Commit logical chunks as you go. Use good commit messages with a one-line
  summary followed by details, in rough Conventional Commits style. Escalate
  privileges to commit.
- Write clean code and refactor when an area becomes complex or difficult to
  understand.
- Prefer principled architecture and fast, optimized implementations. Avoid
  hacks.
- Do not restrict work to narrow or nominally low-risk changes when a clean
  implementation requires an ambitious or cross-cutting design.
- Aim to keep source files under roughly 600 lines when they have a logical
  split. Test files may be longer and should be split by responsibility.
- For complex features, write design or technical documentation in `docs/`
  before implementation. Do not commit temporary task plans or notes.
- Before adding or renaming a Cargo feature, read
  [Cargo Feature Axes](docs/cargo_feature_axes.md). It defines feature
  ownership, axes, and required gate declarations.
- Prefer `#[cfg(test)] mod tests;` with separate `src/.../tests.rs` files for
  nontrivial crate-internal tests. Before writing tests, read
  [Testing Policy](docs/testing_policy.md).
- Keep the routine test suite fast. Use `cargo test -q --tests` to avoid
  doctests and excessive output.
- Limit `rg` output aggressively.
- For `wait` and `write_stdin`, use a timeout of at least 10 minutes. For
  `wait_agent`, use at least 30 minutes.
- Codex: Spawn independent subagents with `fork_turns: "none"`.
- Combine adjacent commands when practical, such as committing only after
  tests pass.
- For canonical-command (`umber2-johp`) semantic or DVI divergence work, first
  read
  [Canonical Divergence Working Contract](docs/canonical_divergence_workflow.md).
  It defines the oracle hierarchy, diagnosis order, fix discipline, and gates.

## Development

- Run relevant tests explicitly, then run `scripts/check.sh` for formatting and
  clippy without rerunning tests.
- `scripts/check.sh` is the authoritative formatting and clippy gate. It runs
  every requested gate even after failures and ends with a verdict naming the
  failures; report that verdict.
- Run an individual authoritative gate by name, such as
  `scripts/check.sh clippy`.
- Do not report an ad hoc `cargo clippy` invocation as the clippy gate. A bare
  invocation can miss warn-level policy lints and feature resolutions covered
  by `scripts/check.sh`.
- `cargo test -q --tests` is the complete routine native suite.
  `default-members` covers every host-testable workspace crate, and
  `default_members_cover_every_host_testable_crate` enforces that coverage.
  `umber-wasm` is the sole omission and declares its separate tier.
- Direct `cargo build` and `cargo check` output to a log file.
- Use `scripts/check-and-test.sh` when the native test suite and
  formatting/clippy gate should run concurrently. Clippy uses
  `target/clippy`, so it does not lock the test build.
- Cargo targets are checkout-local. Persistent linked worktrees retain their
  compiled artifacts.
- Use `cargo run-dev -p umber -- <args>` for local CLI runs that should share
  optimized test artifacts.

### Conformance Assets

Provision gitignored conformance assets after creating or allocating a linked
worktree:

```bash
python3 scripts/provision.py worktree <worktree>
```

The provisioner copies only the `tests/native-test-assets.lock` allowlist from
the primary checkout, verifies every SHA-256 on both sides, and leaves the
copies ignored. Rust tests do not provision their own inputs. Do not manually
link or broadly copy `third_party/`.

If the primary checkout lacks an asset, materialize the pinned TeX Live 2026
sources, oracle, fixtures, and format there once:

```bash
python3 scripts/provision.py worktree .
```

An environment that cannot host the oracles must opt out explicitly with
`UMBER_CONFORMANCE_ORACLES=optional`. This downgrades missing byte-exact
oracles to a notice rather than treating their absence as a pass.

### Snapshot-Sensitive Work

Corpus and format work that depends on snapshots must pass the explicit
regenerated 2026-03-01 distribution, normally:

```text
--distribution target/texlive-snapshot
```

It must also pass the authenticated `--distribution-ahash64` pin and must not
rely on the hosted default manifest.

The native cache is shared and content-addressed rather than
snapshot-partitioned. Stop concurrent Umber runs before purging its `objects`
or `manifests` namespaces, then warm only from the explicit pinned distribution
and verify offline reuse.

### Writing Markdown

`scripts/check.sh` runs dprint over every Markdown file. Its Markdown plugin
rewrites content, so write Markdown in the form dprint already accepts.
`dprint check` is authoritative; do not use ignore directives or plugin
exclusions to silence it.

- Never begin or end an inline code span with a literal space. Use `␣`
  (U+2423 OPEN BOX), so TeX control space is `\␣` and the e-TeX pseudo-file
  trace prefix is `(␣`.
- Keep backticks balanced within each paragraph. Use a double-backtick code
  span for TeX messages containing a literal backtick, such as
  ``You can't use `\eqno' in ... mode``.
- Do not indent fenced-code content beyond its fence.
- Keep each complete `[text](target)` link on one source line.
- Use `_emphasis_`, not `*emphasis*`.

## Beads Issue Tracker

Use the `beads` skill for detailed workflow guidance, then use the `bd` CLI:

```bash
bd ready
bd show <id>
bd update <id> --claim
bd close <id>
```

Use Beads for all task tracking and persistent project memory. Do not create
Markdown TODO lists or ad hoc memory files.

Issues live in a local Dolt database and synchronize through `refs/dolt/data`.
`.beads/issues.jsonl` is a passive export. See the [Beads synchronization documentation](https://github.com/gastownhall/beads/blob/main/docs/SYNC_CONCEPTS.md) for details and anti-patterns.
